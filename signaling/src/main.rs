mod app_state;
mod config;
mod message;
mod peer_registry;
mod redis;
mod sfu;
mod websocket;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::routing::get;
use axum::Router;
use app_state::AppState;
use config::Config;
use peer_registry::PeerRegistry;
use tonic::transport::{Channel, Endpoint};
use tracing::{error, info};
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

    let sfu_channel = connect_sfu(&config.sfu_grpc_addr, config.sfu_connect_timeout).await?;
    info!(addr = %config.sfu_grpc_addr, "Connected to SFU gRPC endpoint");

    let sfu = sfu::Client::new(sfu_channel, config.signaling_id.clone());
    let peers = PeerRegistry::default();

    let state = AppState {
        config: Arc::new(config),
        redis,
        sfu,
        peers,
    };
    let sfu_events = state.sfu.clone();
    let state_for_sfu_events = state.clone();
    tokio::spawn(async move {
        sfu_events.run_event_listener(state_for_sfu_events).await;
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
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

async fn connect_sfu(addr: &str, connect_timeout: Duration) -> Result<Channel> {
    let endpoint = Endpoint::from_shared(addr.to_owned())
        .context("invalid SFU_GRPC_ADDR")?
        .connect_timeout(connect_timeout)
        .tcp_nodelay(true);

    endpoint.connect().await.context("failed to connect to SFU")
}

async fn healthz() -> &'static str {
    "ok"
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
