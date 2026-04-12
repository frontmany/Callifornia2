use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::timeout;
use tracing::warn;

use crate::app_state::AppState;
use crate::message::{ClientMessage, ServerErrorCode, ServerMessage};
use crate::redis::{self, RedisRoomError};
use crate::sfu::SfuClientError;

#[derive(Default)]
struct ConnectionContext {
    room_id: Option<String>,
    nickname: Option<String>,
}

#[derive(Debug, Error)]
enum WsHandlerError {
    #[error("connection is not joined to room")]
    NotInRoom,
    #[error("leave room mismatch")]
    LeaveRoomMismatch,
    #[error("failed to send websocket message")]
    WriteFailed,
    #[error(transparent)]
    Redis(#[from] RedisRoomError),
    #[error(transparent)]
    Sfu(#[from] SfuClientError),
}

pub async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) { 
    let mut heartbeat = tokio::time::interval(state.config.heartbeat_interval);
    let mut ctx = ConnectionContext::default();
    let (outbound_tx, mut outbound_rx) = unbounded_channel::<ServerMessage>();

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let ping_send = timeout(state.config.ws_write_timeout, socket.send(Message::Ping(vec![].into()))).await;
                if ping_send.is_err() || ping_send.ok().and_then(|res| res.err()).is_some() {
                    break;
                }
            }

            incoming = timeout(state.config.ws_read_timeout, socket.next()) => {
                let incoming = match incoming {
                    Ok(msg) => msg,
                    Err(_) => break,
                };

                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(msg) => {
                                if let Err(err) = msg.validate() {
                                    if send_error(
                                        &mut socket,
                                        ServerErrorCode::InvalidPayload,
                                        &err.to_string(),
                                        state.config.ws_write_timeout,
                                    ).await
                                    .is_err()
                                    {
                                        break;
                                    }
                                    continue;
                                }

                                if let Err(err) = handle_client_message(
                                    &mut socket,
                                    &state,
                                    &mut ctx,
                                    &outbound_tx,
                                    msg,
                                ).await {
                                    if matches!(&err, WsHandlerError::WriteFailed) {
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
                            
                            Err(_) => {
                                if send_error(
                                    &mut socket,
                                    ServerErrorCode::InvalidJson,
                                    "Malformed JSON",
                                    state.config.ws_write_timeout,
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            
            outbound = recv_outbound(&mut outbound_rx) => {
                match outbound {
                    Some(payload) => {
                        if send_message(&mut socket, payload, state.config.ws_write_timeout).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    cleanup_connection(&state, &ctx).await;
}

async fn handle_client_message(
    socket: &mut WebSocket,
    state: &AppState,
    ctx: &mut ConnectionContext,
    outbound_tx: &UnboundedSender<ServerMessage>,
    message: ClientMessage,
) -> Result<(), WsHandlerError> {
    match message {
        ClientMessage::Create { nickname } => {
            let room_id = redis::create_room(&state.redis, &nickname).await?;
            state.sfu.ensure_peer(&room_id, &nickname).await?;
            ctx.room_id = Some(room_id.clone());
            ctx.nickname = Some(nickname.clone());
            send_message(
                socket,
                ServerMessage::Created {
                    room_id: room_id.clone(),
                    your_nickname: nickname.clone(),
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            state
                .peers
                .register(&room_id, &nickname, outbound_tx.clone())
                .await;
            Ok(())
        }
        ClientMessage::Join { room_id, nickname } => {
            let participants = redis::join_room(&state.redis, &room_id, &nickname).await?;
            state.sfu.ensure_peer(&room_id, &nickname).await?;
            ctx.room_id = Some(room_id.clone());
            ctx.nickname = Some(nickname.clone());
            send_message(
                socket,
                ServerMessage::Joined {
                    room_id: room_id.clone(),
                    your_nickname: nickname.clone(),
                    participants,
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            state
                .peers
                .register(&room_id, &nickname, outbound_tx.clone())
                .await;
            let stale = state
                .peers
                .broadcast_to_room(
                    &room_id,
                    Some(&nickname),
                    ServerMessage::ParticipantJoined {
                        nickname: nickname.clone(),
                    },
                )
                .await;
            detach_stale_peers(state, &room_id, stale, "stale_sender").await;
            Ok(())
        }
        ClientMessage::Leave { room_id } => {
            let current_room = ctx.room_id.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            let nickname = ctx.nickname.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            if current_room != room_id {
                return Err(WsHandlerError::LeaveRoomMismatch);
            }
            state.detach_peer(&room_id, nickname, "user_left").await;
            let stale = state
                .peers
                .broadcast_to_room(
                    &room_id,
                    Some(nickname),
                    ServerMessage::ParticipantLeft {
                        nickname: nickname.to_owned(),
                    },
                )
                .await;
            detach_stale_peers(state, &room_id, stale, "stale_sender").await;
            ctx.room_id = None;
            ctx.nickname = None;
            Ok(())
        }
        ClientMessage::Sdp { sdp, sdp_type } => {
            let room_id = ctx.room_id.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            let nickname = ctx.nickname.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            let response = state.sfu.handle_sdp(room_id, nickname, sdp, &sdp_type).await?;

            send_message(
                socket,
                ServerMessage::Sdp {
                    from: "sfu".to_owned(),
                    sdp: response.sdp,
                    sdp_type: response.sdp_type,
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            Ok(())
        }
        ClientMessage::Candidate { candidate, sdp_mid } => {
            let room_id = ctx.room_id.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            let nickname = ctx.nickname.as_deref().ok_or(WsHandlerError::NotInRoom)?;
            state
                .sfu
                .add_ice_candidate(room_id, nickname, candidate, sdp_mid)
                .await?;
            Ok(())
        }
    }
}

async fn cleanup_connection(state: &AppState, ctx: &ConnectionContext) {
    let (Some(room_id), Some(nickname)) = (ctx.room_id.as_deref(), ctx.nickname.as_deref()) else {
        return;
    };

    state.detach_peer(room_id, nickname, "signaling_disconnected").await;
    let stale = state
        .peers
        .broadcast_to_room(
            room_id,
            Some(nickname),
            ServerMessage::ParticipantLeft {
                nickname: nickname.to_owned(),
            },
        )
        .await;
    detach_stale_peers(state, room_id, stale, "stale_sender").await;
}

async fn send_error(
    socket: &mut WebSocket,
    code: ServerErrorCode,
    message: &str,
    write_timeout: std::time::Duration,
) -> anyhow::Result<()> {
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

async fn send_message(
    socket: &mut WebSocket,
    payload: ServerMessage,
    write_timeout: std::time::Duration,
) -> anyhow::Result<()> {
    let text = serde_json::to_string(&payload)?;
    timeout(write_timeout, socket.send(Message::Text(text.into()))).await??;
    Ok(())
}

fn log_write_failure(err: anyhow::Error) -> WsHandlerError {
    warn!(error = %err, "websocket send failed");
    WsHandlerError::WriteFailed
}

fn map_error(err: &WsHandlerError) -> (ServerErrorCode, &'static str) {
    match err {
        WsHandlerError::WriteFailed => (ServerErrorCode::Internal, "Failed to send message"),
        WsHandlerError::Redis(room_err) => match room_err {
            RedisRoomError::RoomNotFound => (ServerErrorCode::RoomNotFound, "Room not found"),
            RedisRoomError::NicknameTaken => (ServerErrorCode::NicknameTaken, "Nickname already taken"),
            RedisRoomError::ParticipantNotInRoom => (ServerErrorCode::NotInRoom, "Participant is not in room"),
            RedisRoomError::Redis(_) => (ServerErrorCode::Internal, "Redis operation failed"),
        },
        WsHandlerError::NotInRoom | WsHandlerError::LeaveRoomMismatch => {
            (ServerErrorCode::NotInRoom, "Connection is not joined to this room")
        }
        WsHandlerError::Sfu(SfuClientError::Transport(_)) => {
            (ServerErrorCode::SfuUnavailable, "SFU is unavailable")
        }
        WsHandlerError::Sfu(SfuClientError::Rejected(_)) => {
            (ServerErrorCode::Internal, "SFU rejected signaling request")
        }
    }
}

async fn recv_outbound(outbound_rx: &mut UnboundedReceiver<ServerMessage>) -> Option<ServerMessage> {
    outbound_rx.recv().await
}

async fn detach_stale_peers(state: &AppState, room_id: &str, mut stale_nicknames: Vec<String>, reason: &str) {
    while let Some(nickname) = stale_nicknames.pop() {
        let more_stale = state
            .peers
            .broadcast_to_room(
                room_id,
                Some(&nickname),
                ServerMessage::ParticipantLeft {
                    nickname: nickname.clone(),
                },
            )
            .await;
        state.detach_peer(room_id, &nickname, reason).await;
        stale_nicknames.extend(more_stale);
    }
}