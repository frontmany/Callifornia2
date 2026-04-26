mod config;
mod message;
mod redis;
mod token;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use message::{ClientMessage, ServerErrorCode, ServerMessage};
use thiserror::Error;
use tokio::time::timeout;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Config;

#[derive(Clone)]
struct State {
    config: Arc<Config>,
    redis: ::redis::Client,
}

#[derive(Default)]
struct ConnectionContext {
    session_id: Option<String>,
    nickname: Option<String>,
    handoff_issued: bool,
}

#[derive(Debug, Error)]
enum HandlerError {
    #[error("connection is not authorized")]
    Unauthorized,
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("room not found")]
    RoomNotFound,
    #[error("redis operation failed")]
    Redis,
    #[error("signaling instance is unavailable")]
    SignalingUnavailable,
    #[error("failed to issue connector token")]
    Token,
    #[error("failed to send websocket message")]
    WriteFailed,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    init_tracing();

    let config = Arc::new(Config::from_env().context("load config")?);
    let socket_addr: SocketAddr = config
        .connector_addr
        .parse()
        .with_context(|| format!("invalid CONNECTOR_ADDR: {}", config.connector_addr))?;
    let redis = redis::init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    let state = State { config, redis };

    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .with_state(state);

    info!(%socket_addr, "Connector server started");
    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .context("bind tcp listener")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("run connector server")?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn ws_upgrade(ws: WebSocketUpgrade, AxumState(state): AxumState<State>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: State) {
    let mut context = ConnectionContext::default();
    while let Some(incoming) = socket.next().await {
        let message = match incoming {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        let result = match serde_json::from_str::<ClientMessage>(&message) {
            Ok(message) => {
                if let Err(err) = message.validate() {
                    send_error(
                        &mut socket,
                        ServerErrorCode::InvalidPayload,
                        &err.to_string(),
                        state.config.ws_write_timeout,
                    )
                    .await
                } else {
                    handle_message(&mut socket, &state, &mut context, message).await
                }
            }
            Err(_) => {
                send_error(
                    &mut socket,
                    ServerErrorCode::InvalidJson,
                    "Malformed JSON",
                    state.config.ws_write_timeout,
                )
                .await
            }
        };

        if let Err(err) = result {
            if matches!(err, HandlerError::WriteFailed) {
                break;
            }
            let (code, message) = map_error(&err);
            if send_error(&mut socket, code, message, state.config.ws_write_timeout)
                .await
                .is_err()
            {
                break;
            }
        }
    }

    cleanup_connection(&state, &context).await;
}

async fn handle_message(
    socket: &mut WebSocket,
    state: &State,
    context: &mut ConnectionContext,
    message: ClientMessage,
) -> Result<(), HandlerError> {
    match message {
        ClientMessage::Auth { nickname } => {
            redis::reserve_nickname(&state.redis, &nickname)
                .await
                .map_err(map_redis_error)?;
            let session_id = Uuid::new_v4().to_string();
            if let Err(err) = redis::session_create(&state.redis, &session_id, &nickname).await {
                let _ = redis::release_nickname(&state.redis, &nickname).await;
                return Err(map_redis_error(err));
            }
            context.session_id = Some(session_id.clone());
            context.nickname = Some(nickname.clone());
            send_message(
                socket,
                ServerMessage::AuthOk {
                    nickname,
                    session_id,
                },
                state.config.ws_write_timeout,
            )
            .await
        }
        ClientMessage::Create => {
            let (session_id, nickname) = authenticated_context(context)?;
            let signaling = select_least_loaded_signaling(state).await?;
            let token = issue_signaling_token(
                state,
                &session_id,
                &nickname,
                &signaling.id,
                "create",
                None,
            )?;
            context.handoff_issued = true;
            send_message(
                socket,
                ServerMessage::SignalingReady {
                    signaling_url: signaling.ws_url.clone(),
                    session_id,
                    token,
                },
                state.config.ws_write_timeout,
            )
            .await
        }
        ClientMessage::Join { room_id } => {
            let (session_id, nickname) = authenticated_context(context)?;
            let route = redis::get_room_route(&state.redis, &room_id)
                .await
                .map_err(map_redis_error)?;
            let signaling = state
                .config
                .signaling_by_id(&route.signaling_instance_id)
                .ok_or(HandlerError::SignalingUnavailable)?;
            let token = issue_signaling_token(
                state,
                &session_id,
                &nickname,
                &signaling.id,
                "join",
                Some(room_id),
            )?;
            context.handoff_issued = true;
            send_message(
                socket,
                ServerMessage::SignalingReady {
                    signaling_url: signaling.ws_url.clone(),
                    session_id,
                    token,
                },
                state.config.ws_write_timeout,
            )
            .await
        }
        ClientMessage::Logout => {
            if let (Some(session_id), Some(nickname)) =
                (context.session_id.take(), context.nickname.take())
            {
                let _ = redis::session_delete(&state.redis, &session_id).await;
                let _ = redis::release_nickname(&state.redis, &nickname).await;
            }
            send_message(
                socket,
                ServerMessage::LoggedOut,
                state.config.ws_write_timeout,
            )
            .await
        }
    }
}

async fn cleanup_connection(state: &State, context: &ConnectionContext) {
    if context.handoff_issued {
        return;
    }
    let (Some(session_id), Some(nickname)) = (&context.session_id, &context.nickname) else {
        return;
    };

    if let Err(err) = redis::session_delete(&state.redis, session_id).await {
        warn!(error = %err, session_id = %session_id, "failed to cleanup connector session");
    }
    if let Err(err) = redis::release_nickname(&state.redis, nickname).await {
        warn!(error = %err, nickname = %nickname, "failed to cleanup connector nickname");
    }
}

fn authenticated_context(context: &ConnectionContext) -> Result<(String, String), HandlerError> {
    Ok((
        context
            .session_id
            .clone()
            .ok_or(HandlerError::Unauthorized)?,
        context.nickname.clone().ok_or(HandlerError::Unauthorized)?,
    ))
}

async fn select_least_loaded_signaling(
    state: &State,
) -> Result<&config::SignalingInstance, HandlerError> {
    let loads = redis::load_signaling_loads(&state.redis)
        .await
        .map_err(map_redis_error)?;

    state
        .config
        .signaling_instances
        .iter()
        .min_by(|left, right| {
            let left_load = loads.get(&left.id).copied().unwrap_or_default();
            let right_load = loads.get(&right.id).copied().unwrap_or_default();
            left_load
                .partial_cmp(&right_load)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or(HandlerError::SignalingUnavailable)
}

fn issue_signaling_token(
    state: &State,
    session_id: &str,
    nickname: &str,
    signaling_instance_id: &str,
    intent: &str,
    room_id: Option<String>,
) -> Result<String, HandlerError> {
    token::issue_token(
        &state.config.token_secret,
        state.config.token_ttl,
        session_id.to_owned(),
        nickname.to_owned(),
        signaling_instance_id.to_owned(),
        intent.to_owned(),
        room_id,
    )
    .map_err(|_| HandlerError::Token)
}

async fn send_message(
    socket: &mut WebSocket,
    payload: ServerMessage,
    write_timeout: std::time::Duration,
) -> Result<(), HandlerError> {
    let text = serde_json::to_string(&payload).map_err(|_| HandlerError::WriteFailed)?;
    timeout(write_timeout, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| HandlerError::WriteFailed)?
        .map_err(|_| HandlerError::WriteFailed)
}

async fn send_error(
    socket: &mut WebSocket,
    code: ServerErrorCode,
    message: &str,
    write_timeout: std::time::Duration,
) -> Result<(), HandlerError> {
    send_message(
        socket,
        ServerMessage::Error {
            code,
            message: message.to_owned(),
        },
        write_timeout,
    )
    .await
}

fn map_redis_error(err: redis::RedisError) -> HandlerError {
    match err {
        redis::RedisError::NicknameTaken => HandlerError::NicknameTaken,
        redis::RedisError::RoomNotFound => HandlerError::RoomNotFound,
        redis::RedisError::InvalidRoomRoute | redis::RedisError::Redis(_) => HandlerError::Redis,
    }
}

fn map_error(err: &HandlerError) -> (ServerErrorCode, &'static str) {
    match err {
        HandlerError::Unauthorized => (ServerErrorCode::Unauthorized, "Unauthorized"),
        HandlerError::NicknameTaken => (ServerErrorCode::NicknameTaken, "Nickname already taken"),
        HandlerError::RoomNotFound => (ServerErrorCode::RoomNotFound, "Room not found"),
        HandlerError::Redis => (ServerErrorCode::Internal, "Redis operation failed"),
        HandlerError::SignalingUnavailable => (
            ServerErrorCode::Internal,
            "Signaling instance is unavailable",
        ),
        HandlerError::Token => (ServerErrorCode::Internal, "Failed to issue token"),
        HandlerError::WriteFailed => (ServerErrorCode::Internal, "Failed to send message"),
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to listen for CTRL+C");
    }
    warn!("shutdown signal received");
}
