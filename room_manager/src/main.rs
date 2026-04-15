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
use tracing::info;

use crate::config::Config;
use crate::models::{AppState, RoomManagerServer, StaticInventoryProvider};
use crate::proto::room_manager_pb::room_manager_service_server::RoomManagerServiceServer;
use crate::runtime::health_loop;
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
    let provider = StaticInventoryProvider::new(config.sfu_candidates.clone());

    let state = AppState {
        config,
        redis,
        provider,
    };

    let health_state = state.clone();
    tokio::spawn(async move {
        health_loop(health_state).await;
    });

    info!(addr = %socket_addr, "Room Manager listening");
    Server::builder()
        .add_service(RoomManagerServiceServer::new(RoomManagerServer { state }))
        .serve(socket_addr)
        .await
        .context("room manager server failed")
}
