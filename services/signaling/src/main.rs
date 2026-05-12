mod admin;
mod auth;
mod config;
mod message;
mod peer_registry;
mod redis;
mod session_registry;
mod sfu;
mod state;
mod websocket;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use axum::Router;
use config::Config;
use peer_registry::PeerRegistry;
use serde::Serialize;
use session_registry::SessionRegistry;
use state::DegradationReason;
use state::State;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use websocket::ws_upgrade;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env().with_context(|| "load config error")?;

    let socket_addr: SocketAddr = config
        .signaling_addr
        .parse()
        .with_context(|| format!("invalid SIGNALING_ADDR: {}", config.signaling_addr))?;

    let redis = redis::init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    redis::set_op_timeout(config.redis_op_timeout);
    info!("Redis is reachable");

    let sfu = sfu::Registry::new(
        config.public_node_id(),
        config.rpc_connect_timeout,
        config.sfu_rpc_timeout,
        config.sfu_backoff_min,
        config.sfu_backoff_max,
    );
    let peers = PeerRegistry::default();
    let sessions = SessionRegistry::default();

    let state = State::new(Arc::new(config), redis, sfu, peers, sessions);

    // Start the internal admin gRPC server (supervisor-only).
    let admin_grpc_addr: std::net::SocketAddr =
        state.config.admin_grpc_addr.parse().with_context(|| {
            format!(
                "invalid SIGNALING_ADMIN_GRPC_ADDR: {}",
                state.config.admin_grpc_addr
            )
        })?;
    let admin_state = state.clone();
    tokio::spawn(async move {
        info!(%admin_grpc_addr, "Admin gRPC server starting");
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(admin::make_server(admin_state))
            .serve(admin_grpc_addr)
            .await
        {
            tracing::error!(error = %err, "Admin gRPC server error");
        }
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .with_state(state.clone());

    info!(%socket_addr, "Signaling server started");

    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .context("bind tcp listener")?;

    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            warn!("signaling shutdown: running purge protocol");
            shutdown_state.run_purge(DegradationReason::RedisDown).await;
        })
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

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> (StatusCode, Json<HealthResponse>) {
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
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
