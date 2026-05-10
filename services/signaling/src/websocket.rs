use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::timeout;
use tracing::warn;
use uuid::Uuid;

use crate::auth;
use crate::message::{ClientMessage, ServerErrorCode, ServerMessage};
use crate::redis::{self, RedisRoomError, SessionState};
use crate::sfu::SfuClientError;
use crate::state::State;

#[derive(Default)]
struct ConnectionContext {
    session_id: Option<String>,
    cached_session: Option<SessionState>,
    allowed_intent: Option<String>,
    allowed_room_id: Option<String>,
    instance_load_registered: bool,
    lease_renewer: Option<LeaseRenewerHandle>,
}

struct LeaseRenewerHandle {
    cancel: tokio::sync::oneshot::Sender<()>,
}

impl LeaseRenewerHandle {
    fn stop(self) {
        let _ = self.cancel.send(());
    }
}

#[derive(Debug, Error)]
enum WsHandlerError {
    #[error("connection is already authorized")]
    AlreadyAuthorized,
    #[error("connection is not authorized")]
    Unauthorized,
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
    #[error("invalid room assignment")]
    InvalidRoomAssignment,
    #[error("no ready SFU capacity")]
    SfuCapacityExhausted,
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
) -> axum::response::Response {
    // Reject new connections while a purge is in progress (purging flag set).
    if state.is_purging() {
        return (StatusCode::SERVICE_UNAVAILABLE, "service degraded").into_response();
    }
    ws.on_upgrade(move |socket| handle_ws(socket, state))
        .into_response()
}

async fn handle_ws(mut socket: WebSocket, state: State) {
    // Ping the client at a fixed 30s interval independent of read timeout.
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    let mut context = ConnectionContext::default();
    let (outbound_tx, mut outbound_rx) =
        channel::<ServerMessage>(state.config.peer_outbound_capacity);

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

    cleanup_connection(&state, &mut context).await;
}

async fn handle_client_message(
    socket: &mut WebSocket,
    state: &State,
    context: &mut ConnectionContext,
    outbound_tx: &Sender<ServerMessage>,
    message: ClientMessage,
) -> Result<(), WsHandlerError> {
    match message {
        ClientMessage::Attach { token } => {
            if context.session_id.is_some() {
                return Err(WsHandlerError::AlreadyAuthorized);
            }
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }

            let claims = auth::validate_connector_token(
                &token,
                &state.config.connector_token_secret,
                &state.config.public_node_id(),
            )
            .map_err(|_| WsHandlerError::Unauthorized)?;

            if !claims.jti.is_empty() {
                let jti_ttl = std::time::Duration::from_secs(
                    claims.exp.saturating_sub(claims.iat).max(60) + 30,
                );
                let consumed = redis::consume_jti(&state.redis, &claims.jti, jti_ttl)
                    .await
                    .map_err(map_redis_error)?;
                if !consumed {
                    return Err(WsHandlerError::Unauthorized);
                }
            }

            let session = redis::session_get(&state.redis, &claims.session_id)
                .await
                .map_err(map_redis_error)?
                .filter(|session| session.nickname == claims.nickname)
                .ok_or(WsHandlerError::Unauthorized)?;

            redis::session_set_owner(
                &state.redis,
                &claims.session_id,
                &state.config.public_node_id(),
            )
            .await
            .map_err(map_redis_error)?;

            context.session_id = Some(claims.session_id.clone());
            context.cached_session = Some(session.clone());
            context.allowed_intent = Some(claims.intent);
            context.allowed_room_id = claims.room_id;
            redis::increment_instance_load(&state.redis, &state.config.public_node_id())
                .await
                .map_err(map_redis_error)?;
            context.instance_load_registered = true;
            context.lease_renewer = Some(spawn_lease_renewer(
                state.clone(),
                claims.session_id.clone(),
                session.nickname.clone(),
            ));
            state
                .sessions
                .upsert(&claims.session_id, session.clone())
                .await;
            send_message(
                socket,
                ServerMessage::Attached {
                    nickname: session.nickname,
                    session_id: claims.session_id,
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
                if context.instance_load_registered {
                    redis::decrement_instance_load(&state.redis, &state.config.public_node_id())
                        .await
                        .map_err(map_redis_error)?;
                    context.instance_load_registered = false;
                }
                redis::release_nick_lease(&state.redis, &session.nickname, &session_id)
                    .await
                    .map_err(map_redis_error)?;
                redis::session_delete(&state.redis, &session_id)
                    .await
                    .map_err(map_redis_error)?;
            }
            state.sessions.remove(&session_id).await;
            if let Some(handle) = context.lease_renewer.take() {
                handle.stop();
            }
            context.session_id = None;
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
            let lock_owner = Uuid::new_v4().to_string();
            let acquired = redis::acquire_session_lock(
                &state.redis,
                &session_id,
                &lock_owner,
                state.config.session_lock_ttl,
            )
            .await
            .map_err(map_redis_error)?;
            if !acquired {
                return Err(WsHandlerError::SessionConflict);
            }
            let result =
                handle_create_locked(state, socket, context, outbound_tx, &session_id).await;
            let _ = redis::release_session_lock(&state.redis, &session_id, &lock_owner).await;
            result
        }
        ClientMessage::Join {
            session_id,
            room_id,
        } => {
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }
            let lock_owner = Uuid::new_v4().to_string();
            let acquired = redis::acquire_session_lock(
                &state.redis,
                &session_id,
                &lock_owner,
                state.config.session_lock_ttl,
            )
            .await
            .map_err(map_redis_error)?;
            if !acquired {
                return Err(WsHandlerError::SessionConflict);
            }
            let result =
                handle_join_locked(state, socket, context, outbound_tx, &session_id, room_id).await;
            let _ = redis::release_session_lock(&state.redis, &session_id, &lock_owner).await;
            result
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

async fn cleanup_connection(state: &State, context: &mut ConnectionContext) {
    if let Some(handle) = context.lease_renewer.take() {
        handle.stop();
    }
    let Some(session_id) = context.session_id.as_deref() else {
        return;
    };

    if !state.is_redis_available().await {
        state.sessions.remove(session_id).await;
        return;
    }

    let session = match state.sessions.get(session_id).await {
        Some(session) => Some(session),
        None => match redis::session_get(&state.redis, session_id).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, session_id = %session_id, "failed to load session for cleanup");
                None
            }
        },
    };
    let Some(session) = session else {
        state.sessions.remove(session_id).await;
        return;
    };

    if let Some(room_id) = session.room_id.as_deref() {
        leave_room_and_notify(state, room_id, &session.nickname, "signaling_disconnected").await;
    }
    if context.instance_load_registered {
        if let Err(err) =
            redis::decrement_instance_load(&state.redis, &state.config.public_node_id()).await
        {
            warn!(error = %err, "failed to decrement signaling instance load");
        }
    }
    if let Err(err) = redis::release_nick_lease(&state.redis, &session.nickname, session_id).await {
        warn!(error = %err, nickname = %session.nickname, "failed to release nick lease");
    }
    if let Err(err) = redis::session_delete(&state.redis, session_id).await {
        warn!(error = %err, session_id = %session_id, "failed to delete session");
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
        WsHandlerError::WriteFailed => (ServerErrorCode::WriteFailed, "Failed to send message"),
        WsHandlerError::AlreadyAuthorized => (
            ServerErrorCode::AlreadyAuthorized,
            "Connection is already authorized",
        ),
        WsHandlerError::Unauthorized => (ServerErrorCode::Unauthorized, "Unauthorized"),
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
        WsHandlerError::InvalidRoomAssignment => (
            ServerErrorCode::InvalidRoomAssignment,
            "Invalid room assignment",
        ),
        WsHandlerError::SfuCapacityExhausted => (
            ServerErrorCode::SfuCapacityExhausted,
            "No ready SFU capacity is available",
        ),
        WsHandlerError::RedisError => (
            ServerErrorCode::StorageUnavailable,
            "Storage operation failed",
        ),
        WsHandlerError::NotInRoom => (
            ServerErrorCode::NotInRoom,
            "Connection is not joined to this room",
        ),
        WsHandlerError::SfuUnavailable => (ServerErrorCode::SfuUnavailable, "SFU is unavailable"),
        WsHandlerError::SfuRejected => (
            ServerErrorCode::SfuRejected,
            "SFU rejected signaling request",
        ),
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
            if !state.is_redis_available().await {
                return Err(WsHandlerError::RedisError);
            }
            let session = redis::session_get(&state.redis, session_id)
                .await
                .map_err(map_redis_error)?
                .filter(|session| !session.nickname.is_empty())
                .ok_or(WsHandlerError::SessionConflict)?;

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
        RedisRoomError::NoSfuCapacity => WsHandlerError::SfuCapacityExhausted,
        RedisRoomError::RoomAlreadyExists => WsHandlerError::InvalidRoomAssignment,
        RedisRoomError::InvalidRoomTarget => WsHandlerError::InvalidRoomAssignment,
        RedisRoomError::Timeout | RedisRoomError::Redis(_) => WsHandlerError::RedisError,
    }
}

fn map_sfu_error(err: SfuClientError) -> WsHandlerError {
    match err {
        SfuClientError::Transport(_) | SfuClientError::Connect(_) | SfuClientError::Timeout => {
            WsHandlerError::SfuUnavailable
        }
        SfuClientError::Rejected(_) => WsHandlerError::SfuRejected,
    }
}

fn is_local_target(state: &State, route: &redis::RoomRoute) -> bool {
    route.owner_host == state.config.signaling_public_host
        && route.owner_port == state.config.signaling_public_port
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
    _participants: Option<&[String]>,
    reason: &str,
) -> Result<(), WsHandlerError> {
    if !state.is_redis_available().await {
        return Err(WsHandlerError::RedisError);
    }
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
    redis::session_set_room(&state.redis, session_id, None)
        .await
        .map_err(map_redis_error)?;

    state.sessions.set_room(session_id, None).await;
    if let Some(cached) = context.cached_session.as_mut() {
        cached.room_id = None;
    }
    Ok(())
}

async fn recv_outbound(outbound_rx: &mut Receiver<ServerMessage>) -> Option<ServerMessage> {
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

async fn handle_join_locked(
    state: &State,
    socket: &mut WebSocket,
    context: &mut ConnectionContext,
    outbound_tx: &Sender<ServerMessage>,
    session_id: &str,
    room_id: String,
) -> Result<(), WsHandlerError> {
    let session = check_session(state, context, session_id).await?;
    if session.room_id.is_some() {
        return Err(WsHandlerError::AlreadyInRoom);
    }
    if context.allowed_intent.as_deref() != Some("join")
        || context.allowed_room_id.as_deref() != Some(room_id.as_str())
    {
        return Err(WsHandlerError::Unauthorized);
    }

    let route = redis::get_room_route(&state.redis, &room_id)
        .await
        .map_err(map_redis_error)?;
    if !is_local_target(state, &route) {
        warn!(
            room_id = %room_id,
            expected_host = %state.config.signaling_public_host,
            expected_port = state.config.signaling_public_port,
            actual_host = %route.owner_host,
            actual_port = route.owner_port,
            "connector routed join to a non-owner signaling instance"
        );
        return Err(WsHandlerError::InvalidRoomAssignment);
    }
    if route.room_state != "active" {
        return Err(WsHandlerError::InvalidRoomAssignment);
    }
    let sfu_addr = route.sfu_grpc_addr.ok_or(WsHandlerError::SfuUnavailable)?;

    let participants = redis::join_room(&state.redis, &room_id, &session.nickname)
        .await
        .map_err(map_redis_error)?;
    if let Err(err) = state
        .sfu
        .ensure_peer(state, &sfu_addr, &room_id, &session.nickname)
        .await
    {
        if let Err(srem_err) = redis::srem_member(&state.redis, &room_id, &session.nickname).await {
            warn!(error = %srem_err, room_id = %room_id, "compensating srem failed after sfu ensure_peer");
        }
        return Err(map_sfu_error(err));
    }
    redis::session_set_room(&state.redis, session_id, Some(&room_id))
        .await
        .map_err(map_redis_error)?;
    state
        .sessions
        .set_room(session_id, Some(room_id.clone()))
        .await;
    if let Some(cached) = context.cached_session.as_mut() {
        cached.room_id = Some(room_id.clone());
    }
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
        .register(
            &room_id,
            &session.nickname,
            outbound_tx.clone(),
            Some(sfu_addr.clone()),
        )
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

async fn handle_create_locked(
    state: &State,
    socket: &mut WebSocket,
    context: &mut ConnectionContext,
    outbound_tx: &Sender<ServerMessage>,
    session_id: &str,
) -> Result<(), WsHandlerError> {
    let session = check_session(state, context, session_id).await?;
    if session.room_id.is_some() {
        return Err(WsHandlerError::AlreadyInRoom);
    }
    if context.allowed_intent.as_deref() != Some("create") {
        return Err(WsHandlerError::Unauthorized);
    }

    let room_id = Uuid::new_v4().to_string();
    let assignment = redis::assign_room_to_least_loaded_sfu(
        &state.redis,
        &room_id,
        &state.config.public_node_id(),
        &state.config.signaling_public_host,
        state.config.signaling_public_port,
    )
    .await
    .map_err(map_redis_error)?;
    let sfu_addr = assignment.sfu_grpc_addr;
    if let Err(err) = redis::join_room(&state.redis, &room_id, &session.nickname).await {
        if let Err(close_err) = redis::close_room_binding(&state.redis, &room_id).await {
            warn!(error = %close_err, room_id = %room_id, "failed to close room binding after join failure");
        }
        return Err(map_redis_error(err));
    }
    if let Err(err) = state
        .sfu
        .ensure_peer(state, &sfu_addr, &room_id, &session.nickname)
        .await
    {
        if let Err(srem_err) = redis::srem_member(&state.redis, &room_id, &session.nickname).await {
            warn!(error = %srem_err, room_id = %room_id, "compensating srem failed after sfu ensure_peer");
        }
        if let Err(close_err) = redis::close_room_binding(&state.redis, &room_id).await {
            warn!(error = %close_err, room_id = %room_id, "failed to close room binding after sfu ensure_peer failure");
        }
        return Err(map_sfu_error(err));
    }
    redis::session_set_room(&state.redis, session_id, Some(&room_id))
        .await
        .map_err(map_redis_error)?;
    state
        .sessions
        .set_room(session_id, Some(room_id.clone()))
        .await;
    if let Some(cached) = context.cached_session.as_mut() {
        cached.room_id = Some(room_id.clone());
    }
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
        .register(
            &room_id,
            &session.nickname,
            outbound_tx.clone(),
            Some(sfu_addr.clone()),
        )
        .await;
    Ok(())
}

fn spawn_lease_renewer(state: State, session_id: String, _nickname: String) -> LeaseRenewerHandle {
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
    let renew_interval = state.config.session_renew_interval;
    let session_ttl = state.config.session_ttl;

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(renew_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                _ = ticker.tick() => {
                    if !state.is_redis_available().await {
                        continue;
                    }
                    if let Err(err) =
                        redis::renew_session_ttl(&state.redis, &session_id, session_ttl).await
                    {
                        warn!(error = %err, session_id = %session_id, "failed to renew session ttl");
                    }
                }
            }
        }
    });

    LeaseRenewerHandle { cancel: cancel_tx }
}
