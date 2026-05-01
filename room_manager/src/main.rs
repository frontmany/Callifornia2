mod config;
mod models;
mod proto;
mod runtime;
mod service;
mod storage;
mod util;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::models::{RoomManager, SfuPool, State};
use crate::proto::room_manager_pb::room_manager_service_server::RoomManagerServiceServer;
use crate::runtime::{health_loop, janitor_loop};
use crate::storage::init_redis;
use crate::util::init_tracing;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    init_tracing();

    let config = Arc::new(Config::from_env().context("load room manager config")?);
    let socket_addr: SocketAddr = config
        .grpc_addr
        .parse()
        .with_context(|| format!("invalid ROOM_MANAGER_ADDR: {}", config.grpc_addr))?;
    let redis = init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    crate::storage::set_op_timeout(config.redis_op_timeout);
    let provider = SfuPool::new(config.sfu_candidates.clone());

    let state = State {
        config,
        redis,
        provider,
    };

    let health_state = state.clone();
    tokio::spawn(async move {
        health_loop(health_state).await;
    });

    let janitor_state = state.clone();
    tokio::spawn(async move {
        janitor_loop(janitor_state).await;
    });

    info!(addr = %socket_addr, "Room Manager listening");
    Server::builder()
        .add_service(RoomManagerServiceServer::new(RoomManager { state }))
        .serve_with_shutdown(socket_addr, shutdown_signal())
        .await
        .context("room manager server failed")?;
    info!("Room Manager stopped");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to listen for CTRL+C");
    }
    warn!("room_manager shutdown signal received");
}
