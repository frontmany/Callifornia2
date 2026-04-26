mod auth;
mod config;
mod message;
mod peer_registry;
mod redis;
mod room_manager;
mod session_registry;
mod sfu;
mod state;
mod websocket;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State as AxumState;
use axum::routing::get;
use axum::Json;
use axum::Router;
use config::Config;
use peer_registry::PeerRegistry;
use serde::Serialize;
use session_registry::SessionRegistry;
use state::ServiceState;
use state::State;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use websocket::ws_upgrade;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    init_tracing();

    let config = Config::from_env().with_context(|| "load config error")?;

    let socket_addr: SocketAddr = config
        .signaling_addr
        .parse()
        .with_context(|| format!("invalid SIGNALING_ADDR: {}", config.signaling_addr))?;

    let redis = redis::init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    info!("Redis is reachable");

    let room_manager_channel =
        connect_grpc(&config.room_manager_grpc_addr, config.rpc_connect_timeout).await?;
    info!(addr = %config.room_manager_grpc_addr, "Connected to Room Manager gRPC endpoint");

    let sfu = sfu::Registry::new(config.public_node_id(), config.rpc_connect_timeout);
    let room_manager = room_manager::Client::new(room_manager_channel);
    let peers = PeerRegistry::default();
    let sessions = SessionRegistry::default();

    let state = State::new(Arc::new(config), redis, sfu, room_manager, peers, sessions);

    let redis_monitor_state = state.clone();

    tokio::spawn(async move {
        monitor_redis(redis_monitor_state).await;
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .with_state(state);

    info!(%socket_addr, "Signaling server started");

    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .context("bind tcp listener")?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("run http server")?;

    info!("Signaling server stopped");
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

async fn connect_grpc(addr: &str, connect_timeout: Duration) -> Result<Channel> {
    let endpoint = Endpoint::from_shared(addr.to_owned())
        .with_context(|| format!("invalid gRPC addr: {addr}"))?
        .connect_timeout(connect_timeout)
        .tcp_nodelay(true);

    endpoint
        .connect()
        .await
        .with_context(|| format!("failed to connect to gRPC endpoint: {addr}"))
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    redis: &'static str,
    room_manager: &'static str,
}

async fn health(AxumState(state): AxumState<State>) -> Json<HealthResponse> {
    let health = state.health().await;
    let redis = map_service_state_to_string(health.redis);
    let room_manager = map_service_state_to_string(health.room_manager);
    let status = if matches!(health.redis, ServiceState::Ok)
        && matches!(health.room_manager, ServiceState::Ok)
    {
        "ok"
    } else {
        "degraded"
    };

    Json(HealthResponse {
        status,
        redis,
        room_manager,
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!(error = %err, "failed to listen for CTRL+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                error!(error = %err, "failed to install SIGTERM handler");
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn monitor_redis(state: State) {
    let mut interval = tokio::time::interval(state.config.heartbeat_interval);
    let mut was_down = false;

    loop {
        interval.tick().await;
        match redis::health_ping(&state.redis).await {
            Ok(()) => {
                state.set_redis_state(ServiceState::Ok).await;
                was_down = false;
            }
            Err(err) => {
                if !was_down {
                    warn!(error = %err, "redis health probe failed");
                }
                state.set_redis_state(ServiceState::Down).await;
                was_down = true;
            }
        }
    }
}

fn map_service_state_to_string(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Ok => "ok",
        ServiceState::Down => "down",
    }
}
