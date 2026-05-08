mod config;
mod message;
mod redis;
mod token;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use message::{
    AuthRequest, AuthResponse, CreateRequest, ErrorResponse, JoinRequest, LogoutRequest,
    ServerErrorCode, SignalingReadyResponse,
};
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceState {
    Ok,
    Down,
}
 
#[derive(Clone)]
struct State {
    config: Arc<Config>,
    redis: ::redis::Client,
    redis_state: Arc<RwLock<ServiceState>>,
}

impl State {
    async fn is_redis_available(&self) -> bool {
        *self.redis_state.read().await == ServiceState::Ok
    }

    async fn set_redis_state(&self, state: ServiceState) {
        *self.redis_state.write().await = state;
    }
}

#[derive(Debug)]
enum HandlerError {
    Unauthorized,
    NicknameTaken,
    RoomNotFound,
    Redis,
    SignalingUnavailable,
    UnknownSignalingRoute,
    InvalidPayload(&'static str),
    Token,
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
    redis::set_op_timeout(config.redis_op_timeout);
    let state = State {
        config,
        redis,
        redis_state: Arc::new(RwLock::new(ServiceState::Ok)),
    };

    let monitor_state = state.clone();
    tokio::spawn(async move {
        monitor_redis(monitor_state).await;
    });

    let app = Router::new()
        .route("/auth", post(auth))
        .route("/create", post(create))
        .route("/join", post(join))
        .route("/logout", post(logout))
        .route("/health", get(health))
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

async fn auth(
    AxumState(state): AxumState<State>,
    Json(payload): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    payload
        .validate()
        .map_err(|_| HandlerError::InvalidPayload("Invalid auth request"))?;
    let nickname = payload.nickname.trim().to_owned();
    let session_id = Uuid::new_v4().to_string();

    redis::acquire_nick_lease(
        &state.redis,
        &nickname,
        &session_id,
        state.config.nick_lease_ttl,
    )
    .await
    .map_err(map_redis_error)?;

    if let Err(err) =
        redis::session_create(&state.redis, &session_id, &nickname, state.config.session_ttl).await
    {
        let _ = redis::release_nick_lease(&state.redis, &nickname, &session_id).await;
        return Err(map_redis_error(err).into());
    }

    Ok(Json(AuthResponse {
        nickname,
        session_id,
    }))
}

async fn create(
    AxumState(state): AxumState<State>,
    Json(payload): Json<CreateRequest>,
) -> Result<Json<SignalingReadyResponse>, ApiError> {
    payload
        .validate()
        .map_err(|_| HandlerError::InvalidPayload("Invalid create request"))?;
    let session = redis::session_get(&state.redis, &payload.session_id)
        .await
        .map_err(map_redis_error)?
        .ok_or(HandlerError::Unauthorized)?;
    let lease_ok = redis::extend_nick_lease(
        &state.redis,
        &session.nickname,
        &payload.session_id,
        state.config.nick_lease_ttl,
    )
    .await
    .map_err(map_redis_error)?;
    if !lease_ok {
        return Err(HandlerError::Unauthorized.into());
    }

    let signaling = select_least_loaded_signaling(&state).await?;
    let token = issue_signaling_token(
        &state,
        &payload.session_id,
        &session.nickname,
        &signaling.id,
        "create",
        None,
    )?;

    Ok(Json(SignalingReadyResponse {
        signaling_url: signaling.ws_url.clone(),
        session_id: payload.session_id,
        token,
    }))
}

async fn join(
    AxumState(state): AxumState<State>,
    Json(payload): Json<JoinRequest>,
) -> Result<Json<SignalingReadyResponse>, ApiError> {
    payload
        .validate()
        .map_err(|_| HandlerError::InvalidPayload("Invalid join request"))?;
    let session = redis::session_get(&state.redis, &payload.session_id)
        .await
        .map_err(map_redis_error)?
        .ok_or(HandlerError::Unauthorized)?;
    let lease_ok = redis::extend_nick_lease(
        &state.redis,
        &session.nickname,
        &payload.session_id,
        state.config.nick_lease_ttl,
    )
    .await
    .map_err(map_redis_error)?;
    if !lease_ok {
        return Err(HandlerError::Unauthorized.into());
    }

    let route = redis::get_room_route(&state.redis, &payload.room_id)
        .await
        .map_err(map_redis_error)?;
    let signaling = state
        .config
        .signaling_by_id(&route.signaling_instance_id)
        .ok_or(HandlerError::UnknownSignalingRoute)?;
    let token = issue_signaling_token(
        &state,
        &payload.session_id,
        &session.nickname,
        &signaling.id,
        "join",
        Some(payload.room_id),
    )?;

    Ok(Json(SignalingReadyResponse {
        signaling_url: signaling.ws_url.clone(),
        session_id: payload.session_id,
        token,
    }))
}

async fn logout(
    AxumState(state): AxumState<State>,
    Json(payload): Json<LogoutRequest>,
) -> Result<StatusCode, ApiError> {
    payload
        .validate()
        .map_err(|_| HandlerError::InvalidPayload("Invalid logout request"))?;
    let session = redis::session_get(&state.redis, &payload.session_id)
        .await
        .map_err(map_redis_error)?;
    if let Some(session) = session {
        let _ = redis::session_delete(&state.redis, &payload.session_id).await;
        let _ = redis::release_nick_lease(&state.redis, &session.nickname, &payload.session_id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn select_least_loaded_signaling(
    state: &State,
) -> Result<&config::SignalingInstance, HandlerError> {
    let loads = redis::load_signaling_loads(&state.redis)
        .await
        .map_err(map_redis_error)?;
    let alive = redis::alive_signaling_nodes(&state.redis, state.config.signaling_stale_timeout)
        .await
        .map_err(map_redis_error)?;
    let alive_set: std::collections::HashSet<&str> =
        alive.iter().map(String::as_str).collect();

    state
        .config
        .signaling_instances
        .iter()
        .filter(|instance| alive_set.contains(instance.id.as_str()))
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

fn map_redis_error(err: redis::RedisError) -> HandlerError {
    match err {
        redis::RedisError::NicknameTaken => HandlerError::NicknameTaken,
        redis::RedisError::RoomNotFound => HandlerError::RoomNotFound,
        redis::RedisError::InvalidRoomRoute
        | redis::RedisError::Timeout
        | redis::RedisError::Redis(_) => HandlerError::Redis,
    }
}

fn map_error(err: &HandlerError) -> (ServerErrorCode, &'static str) {
    match err {
        HandlerError::Unauthorized => (ServerErrorCode::Unauthorized, "Unauthorized session"),
        HandlerError::NicknameTaken => (ServerErrorCode::NicknameTaken, "Nickname already taken"),
        HandlerError::RoomNotFound => (ServerErrorCode::RoomNotFound, "Room not found"),
        HandlerError::Redis => (ServerErrorCode::StorageUnavailable, "Storage operation failed"),
        HandlerError::SignalingUnavailable => (
            ServerErrorCode::NoHealthySignaling,
            "No healthy signaling instance",
        ),
        HandlerError::UnknownSignalingRoute => (
            ServerErrorCode::UnknownSignalingRoute,
            "Signaling route is not configured on this connector",
        ),
        HandlerError::Token => (ServerErrorCode::TokenIssueFailed, "Failed to issue token"),
        HandlerError::InvalidPayload(message) => (ServerErrorCode::InvalidPayload, message),
    }
}

fn status_for_error(err: &HandlerError) -> StatusCode {
    match err {
        HandlerError::Unauthorized => StatusCode::UNAUTHORIZED,
        HandlerError::NicknameTaken => StatusCode::CONFLICT,
        HandlerError::RoomNotFound => StatusCode::NOT_FOUND,
        HandlerError::Redis => StatusCode::SERVICE_UNAVAILABLE,
        HandlerError::SignalingUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        HandlerError::UnknownSignalingRoute => StatusCode::SERVICE_UNAVAILABLE,
        HandlerError::InvalidPayload(_) => StatusCode::BAD_REQUEST,
        HandlerError::Token => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

struct ApiError(HandlerError);

impl From<HandlerError> for ApiError {
    fn from(value: HandlerError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = status_for_error(&self.0);
        let (code, message) = map_error(&self.0);
        (
            status,
            Json(ErrorResponse {
                code,
                message: message.to_owned(),
            }),
        )
            .into_response()
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to listen for CTRL+C");
    }
    warn!("shutdown signal received");
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    redis: &'static str,
}

async fn health(AxumState(state): AxumState<State>) -> (StatusCode, Json<HealthResponse>) {
    let redis_ok = state.is_redis_available().await;
    let code = if redis_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(HealthResponse {
            status: if redis_ok { "ok" } else { "degraded" },
            redis: if redis_ok { "ok" } else { "down" },
        }),
    )
}

async fn monitor_redis(state: State) {
    let mut interval = tokio::time::interval(state.config.redis_probe_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut was_down = false;

    loop {
        interval.tick().await;
        match redis::health_ping(&state.redis).await {
            Ok(()) => {
                if was_down {
                    info!("connector redis recovered");
                }
                state.set_redis_state(ServiceState::Ok).await;
                was_down = false;
            }
            Err(err) => {
                if !was_down {
                    warn!(error = %err, "connector redis health probe failed");
                }
                state.set_redis_state(ServiceState::Down).await;
                was_down = true;
            }
        }
    }
}
