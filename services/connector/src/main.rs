mod config;
mod message;
mod redis;
mod token;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State as AxumState;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::cors::{AllowOrigin, CorsLayer};
use message::{
    AuthRequest, AuthResponse, CreateRequest, ErrorResponse, JoinRequest, LogoutRequest,
    ServerErrorCode, SessionRenewRequest, SignalingReadyResponse,
};
use serde::Serialize;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Config;

#[derive(Clone)]
struct State {
    config: Arc<Config>,
    redis: ::redis::Client,
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
    SupervisorUnavailable,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Arc::new(Config::from_env().context("load config")?);

    let socket_addr: SocketAddr = config
        .connector_addr
        .parse()
        .with_context(|| format!("invalid CONNECTOR_ADDR: {}", config.connector_addr))?;

    let redis = redis::init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    redis::set_op_timeout(config.redis_op_timeout);
    let state = State {
        config: config.clone(),
        redis,
    };

    let mut app = Router::new()
        .route("/auth", post(auth))
        .route("/create", post(create))
        .route("/join", post(join))
        .route("/logout", post(logout))
        .route("/session/renew", post(session_renew))
        .route("/health", get(health))
        .with_state(state);

    if let Some(layer) = cors_layer(&config.cors_allowed_origins) {
        app = app.layer(layer);
    }

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

fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let values: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin.trim()).ok())
        .collect();
    if values.is_empty() {
        return None;
    }

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(values))
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE])
            .max_age(std::time::Duration::from_secs(600)),
    )
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

    redis::reserve_nickname(&state.redis, &nickname, &session_id)
        .await
        .map_err(map_redis_error)?;

    if let Err(err) = redis::session_create(
        &state.redis,
        &session_id,
        &nickname,
        state.config.session_ttl,
    )
    .await
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

    // Reject new room creation when supervisor is stale — room cleanup won't work.
    check_supervisor_freshness(&state)
        .await
        .map_err(|_| HandlerError::SupervisorUnavailable)?;

    let session = redis::session_get(&state.redis, &payload.session_id)
        .await
        .map_err(map_redis_error)?
        .ok_or(HandlerError::Unauthorized)?;

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
        let _ =
            redis::release_nick_lease(&state.redis, &session.nickname, &payload.session_id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn session_renew(
    AxumState(state): AxumState<State>,
    Json(payload): Json<SessionRenewRequest>,
) -> Result<StatusCode, ApiError> {
    payload
        .validate()
        .map_err(|_| HandlerError::InvalidPayload("Invalid session renew request"))?;
    let session = redis::session_get(&state.redis, &payload.session_id)
        .await
        .map_err(map_redis_error)?
        .ok_or(HandlerError::Unauthorized)?;
    let owner = redis::nick_owner_get(&state.redis, &session.nickname)
        .await
        .map_err(map_redis_error)?;
    if owner.as_deref() != Some(payload.session_id.as_str()) {
        return Err(HandlerError::Unauthorized.into());
    }
    let ok =
        redis::session_refresh_ttl(&state.redis, &payload.session_id, state.config.session_ttl)
            .await
            .map_err(map_redis_error)?;
    if !ok {
        return Err(HandlerError::Unauthorized.into());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn check_supervisor_freshness(state: &State) -> Result<(), HandlerError> {
    let status = redis::get_supervisor_status(&state.redis)
        .await
        .map_err(map_redis_error)?;
    let Some(status) = status else {
        return Err(HandlerError::SupervisorUnavailable);
    };
    let stale_sec = state.config.supervisor_stale_sec as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if now - status.last_seen_unix > stale_sec {
        return Err(HandlerError::SupervisorUnavailable);
    }
    Ok(())
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
    let alive_set: std::collections::HashSet<&str> = alive.iter().map(String::as_str).collect();

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
        HandlerError::Redis => (
            ServerErrorCode::StorageUnavailable,
            "Storage operation failed",
        ),
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
        HandlerError::SupervisorUnavailable => (
            ServerErrorCode::StorageUnavailable,
            "Supervisor not available; retry shortly",
        ),
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
        HandlerError::SupervisorUnavailable => StatusCode::SERVICE_UNAVAILABLE,
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
}

async fn health() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}
