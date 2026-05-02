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
use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use axum::Router;
use config::Config;
use peer_registry::PeerRegistry;
use serde::Serialize;
use session_registry::SessionRegistry;
use state::DegradationReason;
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
    redis::set_op_timeout(config.redis_op_timeout);
    info!("Redis is reachable");

    let room_manager_channel =
        connect_grpc(&config.room_manager_grpc_addr, config.rpc_connect_timeout).await?;
    info!(addr = %config.room_manager_grpc_addr, "Connected to Room Manager gRPC endpoint");

    let sfu = sfu::Registry::new(
        config.public_node_id(),
        config.rpc_connect_timeout,
        config.sfu_rpc_timeout,
        config.sfu_backoff_min,
        config.sfu_backoff_max,
    );
    let room_manager = room_manager::Client::new(room_manager_channel);
    let peers = PeerRegistry::default();
    let sessions = SessionRegistry::default();

    let state = State::new(Arc::new(config), redis, sfu, room_manager, peers, sessions);

    let redis_monitor_state = state.clone();
    tokio::spawn(async move {
        monitor_redis(redis_monitor_state).await;
    });

    let rm_monitor_state = state.clone();
    tokio::spawn(async move {
        monitor_room_manager(rm_monitor_state).await;
    });

    let hb_state = state.clone();
    tokio::spawn(async move {
        heartbeat_loop(hb_state).await;
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
            shutdown_state
                .run_purge(DegradationReason::RoomManagerDown)
                .await;
            let node_id = shutdown_state.config.public_node_id();
            if let Err(err) =
                redis::remove_node_heartbeat(&shutdown_state.redis, &node_id).await
            {
                warn!(error = %err, "failed to remove node heartbeat on shutdown");
            }
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

async fn connect_grpc(addr: &str, connect_timeout: Duration) -> Result<Channel> {
    let endpoint = Endpoint::from_shared(addr.to_owned())
        .with_context(|| format!("invalid gRPC addr: {addr}"))?
        .connect_timeout(connect_timeout)
        .tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .http2_keep_alive_interval(Duration::from_secs(20))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true);

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

async fn health(
    AxumState(state): AxumState<State>,
) -> (StatusCode, Json<HealthResponse>) {
    let health = state.health().await;
    let redis = map_service_state_to_string(health.redis);
    let room_manager = map_service_state_to_string(health.room_manager);
    let healthy = matches!(health.redis, ServiceState::Ok)
        && matches!(health.room_manager, ServiceState::Ok);
    let status = if healthy { "ok" } else { "degraded" };
    let code = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        code,
        Json(HealthResponse {
            status,
            redis,
            room_manager,
        }),
    )
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
                if was_down {
                    info!("redis recovered");
                    state.end_purge();
                }
                state.set_redis_state(ServiceState::Ok).await;
                was_down = false;
            }
            Err(err) => {
                if !was_down {
                    warn!(error = %err, "redis health probe failed");
                    state.set_redis_state(ServiceState::Down).await;
                    let purge_state = state.clone();
                    tokio::spawn(async move {
                        purge_state.run_purge(DegradationReason::RedisDown).await;
                    });
                } else {
                    state.set_redis_state(ServiceState::Down).await;
                }
                was_down = true;
            }
        }
    }
}

async fn heartbeat_loop(state: State) {
    let mut interval = tokio::time::interval(state.config.signaling_heartbeat);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let node_id = state.config.public_node_id();

    loop {
        interval.tick().await;
        if !state.is_healthy().await {
            continue;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if let Err(err) = redis::write_node_heartbeat(&state.redis, &node_id, now).await {
            warn!(error = %err, "failed to write signaling heartbeat");
        }
    }
}

async fn monitor_room_manager(state: State) {
    let mut interval = tokio::time::interval(state.config.room_manager_probe_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let probe_timeout = state.config.room_manager_probe_timeout;
    let mut was_down = false;

    loop {
        interval.tick().await;
        let result =
            tokio::time::timeout(probe_timeout, state.room_manager.get_status()).await;
        let healthy = matches!(result, Ok(Ok(_)));
        if healthy {
            if was_down {
                info!("room_manager recovered");
                state.end_purge();
            }
            state.set_room_manager_state(ServiceState::Ok).await;
            was_down = false;
        } else {
            if !was_down {
                match &result {
                    Ok(Err(err)) => warn!(error = %err, "room_manager health probe failed"),
                    Err(_) => warn!("room_manager health probe timeout"),
                    _ => {}
                }
                state.set_room_manager_state(ServiceState::Down).await;
                let purge_state = state.clone();
                tokio::spawn(async move {
                    purge_state
                        .run_purge(DegradationReason::RoomManagerDown)
                        .await;
                });
            } else {
                state.set_room_manager_state(ServiceState::Down).await;
            }
            was_down = true;
        }
    }
}

fn map_service_state_to_string(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Ok => "ok",
        ServiceState::Down => "down",
    }
}
