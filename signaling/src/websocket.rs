use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::response::IntoResponse;
use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::timeout;
use tracing::warn;
use uuid::Uuid;

use crate::message::{ClientMessage, ServerErrorCode, ServerMessage};
use crate::redis::{self, RedisRoomError, RoomTarget, SessionState};
use crate::room_manager::{AssignmentStatus, RoomManagerError};
use crate::sfu::SfuClientError;
use crate::state::State;

#[derive(Default)]
struct ConnectionContext {
    session_id: Option<String>,
    keep_session_for_transfer: bool,
    cached_session: Option<SessionState>,
}

#[derive(Debug, Error)]
enum WsHandlerError {
    #[error("connection is already authorized")]
    AlreadyAuthorized,
    #[error("session conflict")]
    SessionConflict,
    #[error("connection is not joined to room")]
    NotInRoom,
    #[error("leave room mismatch")]
    LeaveRoomMismatch,
    #[error("connection already in room")]
    AlreadyInRoom,
    #[error("room not found")]
    RoomNotFound,
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("participant is not in room")]
    ParticipantNotInRoom,
    #[error("redis operation failed")]
    RedisError,
    #[error("room transfer target is unavailable")]
    TransferUnavailable,
    #[error("room is not ready")]
    RoomNotReady,
    #[error("sfu is unavailable")]
    SfuUnavailable,
    #[error("sfu rejected signaling request")]
    SfuRejected,
    #[error("failed to send websocket message")]
    WriteFailed,
}

pub async fn ws_upgrade(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<State>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: State) {
    let mut heartbeat = tokio::time::interval(state.config.heartbeat_interval);
    let mut context = ConnectionContext::default();
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
                                    &mut context,
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

    cleanup_connection(&state, &context).await;
}

async fn handle_client_message(
    socket: &mut WebSocket,
    state: &State,
    context: &mut ConnectionContext,
    outbound_tx: &UnboundedSender<ServerMessage>,
    message: ClientMessage,
) -> Result<(), WsHandlerError> {
    match message {
        ClientMessage::Auth { nickname } => {
            if context.session_id.is_some() {
                return Err(WsHandlerError::AlreadyAuthorized);
            }
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }

            redis::reserve_nickname(&state.redis, &nickname)
                .await
                .map_err(map_redis_error)?;
            let session_id = Uuid::new_v4().to_string();
            if let Err(err) = redis::session_create(&state.redis, &session_id, &nickname).await {
                let _ = redis::release_nickname(&state.redis, &nickname).await;
                return Err(map_redis_error(err));
            }

            context.session_id = Some(session_id.clone());
            context.keep_session_for_transfer = false;
            context.cached_session = Some(SessionState {
                nickname: nickname.clone(),
                room_id: None,
                pending_room_id: None,
            });
            state
                .sessions
                .upsert(
                    &session_id,
                    SessionState {
                        nickname: nickname.clone(),
                        room_id: None,
                        pending_room_id: None,
                    },
                )
                .await;
            send_message(
                socket,
                ServerMessage::AuthOk {
                    nickname,
                    session_id,
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            Ok(())
        }
        ClientMessage::Logout {
            session_id,
            participants,
        } => {
            let session = check_session(state, context, &session_id).await?;
            if let Some(room_id) = session.room_id.as_deref() {
                leave_room(
                    state,
                    context,
                    &session_id,
                    room_id,
                    &session.nickname,
                    participants.as_deref(),
                    "user_logout",
                )
                .await?;
            }
            if state.is_redis_available().await {
                redis::release_nickname(&state.redis, &session.nickname)
                    .await
                    .map_err(map_redis_error)?;
                redis::session_delete(&state.redis, &session_id)
                    .await
                    .map_err(map_redis_error)?;
            }
            state.sessions.remove(&session_id).await;
            context.session_id = None;
            context.keep_session_for_transfer = false;
            context.cached_session = None;
            send_message(
                socket,
                ServerMessage::LoggedOut {
                    nickname: session.nickname,
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            Ok(())
        }
        ClientMessage::Create { session_id } => {
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }
            let session = check_session(state, context, &session_id).await?;
            if session.room_id.is_some() {
                return Err(WsHandlerError::AlreadyInRoom);
            }

            let room_id = session
                .pending_room_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let (assignment_status, assignment) = state
                .room_manager
                .get_room(
                    &room_id,
                    &state.config.public_node_id(),
                    &state.config.signaling_public_host,
                    state.config.signaling_public_port,
                    true,
                )
                .await
                .map_err(map_room_manager_error)?;
            if assignment_status != AssignmentStatus::Assigned || assignment.room_state != "active"
            {
                redis::session_set_pending_room(&state.redis, &session_id, Some(&room_id))
                    .await
                    .map_err(map_redis_error)?;
                state
                    .sessions
                    .set_pending_room(&session_id, Some(room_id.clone()))
                    .await;
                if let Some(cached) = context.cached_session.as_mut() {
                    cached.pending_room_id = Some(room_id);
                }
                return Err(WsHandlerError::RoomNotReady);
            }
            let sfu_addr = assignment
                .sfu_grpc_addr
                .ok_or(WsHandlerError::SfuUnavailable)?;

            let _ = redis::join_room(&state.redis, &room_id, &session.nickname)
                .await
                .map_err(map_redis_error)?;
            state
                .sfu
                .ensure_peer(state, &sfu_addr, &room_id, &session.nickname)
                .await
                .map_err(map_sfu_error)?;
            redis::session_set_pending_room(&state.redis, &session_id, None)
                .await
                .map_err(map_redis_error)?;
            redis::session_set_room(&state.redis, &session_id, Some(&room_id))
                .await
                .map_err(map_redis_error)?;
            state
                .sessions
                .set_room(&session_id, Some(room_id.clone()))
                .await;
            state.sessions.set_pending_room(&session_id, None).await;
            if let Some(cached) = context.cached_session.as_mut() {
                cached.room_id = Some(room_id.clone());
                cached.pending_room_id = None;
            }
            context.keep_session_for_transfer = false;
            send_message(
                socket,
                ServerMessage::Created {
                    room_id: room_id.clone(),
                    your_nickname: session.nickname.clone(),
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            state
                .peers
                .register(&room_id, &session.nickname, outbound_tx.clone())
                .await;
            Ok(())
        }
        ClientMessage::Join {
            session_id,
            room_id,
        } => {
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }
            let session = check_session(state, context, &session_id).await?;
            if session.room_id.is_some() {
                return Err(WsHandlerError::AlreadyInRoom);
            }

            let route = redis::get_room_route(&state.redis, &room_id)
                .await
                .map_err(map_redis_error)?;
            let target = route.target.clone();
            if !is_local_target(state, &target) {
                context.keep_session_for_transfer = true;
                send_message(
                    socket,
                    ServerMessage::TransferRequired {
                        room_id,
                        target_host: target.host,
                        target_port: target.port,
                    },
                    state.config.ws_write_timeout,
                )
                .await
                .map_err(log_write_failure)?;
                return Ok(());
            }
            if route.room_state != "active" {
                return Err(WsHandlerError::RoomNotReady);
            }
            let sfu_addr = route.sfu_grpc_addr.ok_or(WsHandlerError::SfuUnavailable)?;

            let participants = redis::join_room(&state.redis, &room_id, &session.nickname)
                .await
                .map_err(map_redis_error)?;
            state
                .sfu
                .ensure_peer(state, &sfu_addr, &room_id, &session.nickname)
                .await
                .map_err(map_sfu_error)?;
            redis::session_set_room(&state.redis, &session_id, Some(&room_id))
                .await
                .map_err(map_redis_error)?;
            state
                .sessions
                .set_room(&session_id, Some(room_id.clone()))
                .await;
            if let Some(cached) = context.cached_session.as_mut() {
                cached.room_id = Some(room_id.clone());
            }
            context.keep_session_for_transfer = false;
            send_message(
                socket,
                ServerMessage::Joined {
                    room_id: room_id.clone(),
                    your_nickname: session.nickname.clone(),
                    participants,
                },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            state
                .peers
                .register(&room_id, &session.nickname, outbound_tx.clone())
                .await;
            let stale = state
                .peers
                .broadcast_to_room(
                    &room_id,
                    Some(&session.nickname),
                    ServerMessage::ParticipantJoined {
                        nickname: session.nickname.clone(),
                    },
                )
                .await;
            detach_stale_peers(state, &room_id, stale, "stale_sender").await;
            Ok(())
        }
        ClientMessage::Leave {
            session_id,
            room_id,
            participants,
        } => {
            let session = check_session(state, context, &session_id).await?;
            let current_room = session
                .room_id
                .as_deref()
                .ok_or(WsHandlerError::NotInRoom)?;
            if current_room != room_id {
                return Err(WsHandlerError::LeaveRoomMismatch);
            }

            leave_room(
                state,
                context,
                &session_id,
                &room_id,
                &session.nickname,
                participants.as_deref(),
                "user_left",
            )
            .await?;
            send_message(
                socket,
                ServerMessage::Left { room_id },
                state.config.ws_write_timeout,
            )
            .await
            .map_err(log_write_failure)?;
            Ok(())
        }
        ClientMessage::Sdp {
            session_id,
            sdp,
            sdp_type,
        } => {
            let session = check_session(state, context, &session_id).await?;
            let room_id = session
                .room_id
                .as_deref()
                .ok_or(WsHandlerError::NotInRoom)?;
            let route = redis::get_room_route(&state.redis, room_id)
                .await
                .map_err(map_redis_error)?;
            let sfu_addr = route.sfu_grpc_addr.ok_or(WsHandlerError::SfuUnavailable)?;
            let response = state
                .sfu
                .handle_sdp(&sfu_addr, room_id, &session.nickname, sdp, &sdp_type)
                .await
                .map_err(map_sfu_error)?;

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
        ClientMessage::Candidate {
            session_id,
            candidate,
            sdp_mid,
        } => {
            let session = check_session(state, context, &session_id).await?;
            let room_id = session
                .room_id
                .as_deref()
                .ok_or(WsHandlerError::NotInRoom)?;
            let route = redis::get_room_route(&state.redis, room_id)
                .await
                .map_err(map_redis_error)?;
            let sfu_addr = route.sfu_grpc_addr.ok_or(WsHandlerError::SfuUnavailable)?;
            state
                .sfu
                .add_ice_candidate(&sfu_addr, room_id, &session.nickname, candidate, sdp_mid)
                .await
                .map_err(map_sfu_error)?;
            Ok(())
        }
    }
}

async fn cleanup_connection(state: &State, context: &ConnectionContext) {
    let Some(session_id) = context.session_id.as_deref() else {
        return;
    };

    if context.keep_session_for_transfer {
        state.sessions.remove(session_id).await;
        return;
    }

    let session = match state.sessions.get(session_id).await {
        Some(session) => Some(session),
        None => context.cached_session.clone(),
    };
    let Some(session) = session else {
        return;
    };

    if let Some(room_id) = session.room_id.as_deref() {
        leave_room_and_notify(state, room_id, &session.nickname, "signaling_disconnected").await;
    }
    if state.is_redis_available().await {
        if let Err(err) = redis::release_nickname(&state.redis, &session.nickname).await {
            warn!(error = %err, nickname = %session.nickname, "failed to release auth nickname");
        }
        if let Err(err) = redis::session_delete(&state.redis, session_id).await {
            warn!(error = %err, session_id = %session_id, "failed to delete session");
        }
    }
    state.sessions.remove(session_id).await;
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
        WsHandlerError::AlreadyAuthorized => (
            ServerErrorCode::AlreadyAuthorized,
            "Connection is already authorized",
        ),
        WsHandlerError::SessionConflict => (
            ServerErrorCode::SessionConflict,
            "Invalid or conflicting session",
        ),
        WsHandlerError::LeaveRoomMismatch => (
            ServerErrorCode::LeaveRoomMismatch,
            "Leave room does not match active room",
        ),
        WsHandlerError::AlreadyInRoom => (
            ServerErrorCode::AlreadyInRoom,
            "Connection already belongs to a room",
        ),
        WsHandlerError::RoomNotFound => (ServerErrorCode::RoomNotFound, "Room not found"),
        WsHandlerError::NicknameTaken => (ServerErrorCode::NicknameTaken, "Nickname already taken"),
        WsHandlerError::ParticipantNotInRoom => {
            (ServerErrorCode::NotInRoom, "Participant is not in room")
        }
        WsHandlerError::RoomNotReady => (
            ServerErrorCode::RoomNotReady,
            "Room is still being assigned to an SFU; retry later",
        ),
        WsHandlerError::RedisError => (ServerErrorCode::Internal, "Redis operation failed"),
        WsHandlerError::TransferUnavailable => (
            ServerErrorCode::TransferUnavailable,
            "Room transfer target is unavailable",
        ),
        WsHandlerError::NotInRoom => (
            ServerErrorCode::NotInRoom,
            "Connection is not joined to this room",
        ),
        WsHandlerError::SfuUnavailable => (ServerErrorCode::SfuUnavailable, "SFU is unavailable"),
        WsHandlerError::SfuRejected => {
            (ServerErrorCode::Internal, "SFU rejected signaling request")
        }
    }
}

async fn check_session(
    state: &State,
    context: &mut ConnectionContext,
    session_id: &str,
) -> Result<SessionState, WsHandlerError> {
    match context.session_id.as_deref() {
        Some(bound) if bound != session_id => Err(WsHandlerError::SessionConflict),
        Some(_) | None => {
            let session = if state.is_redis_available().await {
                match redis::session_get(&state.redis, session_id)
                    .await
                    .map_err(map_redis_error)?
                    .filter(|session| !session.nickname.is_empty())
                {
                    Some(session) => session,
                    None => {
                        let local = state
                            .sessions
                            .get(session_id)
                            .await
                            .or(context.cached_session.clone())
                            .ok_or(WsHandlerError::SessionConflict)?;
                        redis::session_store(&state.redis, session_id, &local)
                            .await
                            .map_err(map_redis_error)?;
                        local
                    }
                }
            } else {
                state
                    .sessions
                    .get(session_id)
                    .await
                    .or(context.cached_session.clone())
                    .ok_or(WsHandlerError::SessionConflict)?
            };

            context.session_id = Some(session_id.to_owned());
            context.cached_session = Some(session.clone());
            state.sessions.upsert(session_id, session.clone()).await;
            Ok(session)
        }
    }
}

fn map_redis_error(err: RedisRoomError) -> WsHandlerError {
    match err {
        RedisRoomError::RoomNotFound => WsHandlerError::RoomNotFound,
        RedisRoomError::NicknameTaken => WsHandlerError::NicknameTaken,
        RedisRoomError::ParticipantNotInRoom => WsHandlerError::ParticipantNotInRoom,
        RedisRoomError::InvalidRoomTarget => WsHandlerError::TransferUnavailable,
        RedisRoomError::Redis(_) => WsHandlerError::RedisError,
    }
}

fn map_sfu_error(err: SfuClientError) -> WsHandlerError {
    match err {
        SfuClientError::Transport(_) | SfuClientError::Connect(_) => WsHandlerError::SfuUnavailable,
        SfuClientError::Rejected(_) => WsHandlerError::SfuRejected,
    }
}

fn map_room_manager_error(err: RoomManagerError) -> WsHandlerError {
    match err {
        RoomManagerError::Transport(_) | RoomManagerError::InvalidAssignment => {
            WsHandlerError::RoomNotReady
        }
    }
}

fn is_local_target(state: &State, target: &RoomTarget) -> bool {
    target.host == state.config.signaling_public_host
        && target.port == state.config.signaling_public_port
}

async fn leave_room_and_notify(state: &State, room_id: &str, nickname: &str, reason: &str) {
    state.detach_peer(room_id, nickname, reason).await;
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

async fn leave_room(
    state: &State,
    context: &mut ConnectionContext,
    session_id: &str,
    room_id: &str,
    nickname: &str,
    participants: Option<&[String]>,
    reason: &str,
) -> Result<(), WsHandlerError> {
    state.detach_peer(room_id, nickname, reason).await;

    if state.is_redis_available().await {
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
        redis::session_set_room(&state.redis, session_id, None)
            .await
            .map_err(map_redis_error)?;
    } else if let Some(participants) = participants {
        state
            .notify_participants_left(room_id, nickname, participants)
            .await;
    }

    state.sessions.set_room(session_id, None).await;
    if let Some(cached) = context.cached_session.as_mut() {
        cached.room_id = None;
    }
    Ok(())
}

async fn recv_outbound(
    outbound_rx: &mut UnboundedReceiver<ServerMessage>,
) -> Option<ServerMessage> {
    outbound_rx.recv().await
}

async fn detach_stale_peers(
    state: &State,
    room_id: &str,
    mut stale_nicknames: Vec<String>,
    reason: &str,
) {
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
