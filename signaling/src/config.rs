use std::env;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    /// Идентификатор инстанса signaling для SFU (CreatePeer, SubscribeEvents).
    pub signaling_id: String,
    pub signaling_addr: String,
    pub redis_url: String,
    pub sfu_grpc_addr: String,
    pub redis_connect_timeout: Duration,
    pub sfu_connect_timeout: Duration,
    pub ws_read_timeout: Duration,
    pub ws_write_timeout: Duration,
    pub heartbeat_interval: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let signaling_id = get_env_or("SIGNALING_ID", "signaling-rust");
        let signaling_addr = get_env_or("SIGNALING_ADDR", "0.0.0.0:8080");
        let redis_url = get_env_or("REDIS_URL", "redis://redis:6379/");
        let sfu_grpc_addr = get_env_or("SFU_GRPC_ADDR", "http://sfu:9090");

        let redis_connect_timeout =
            duration_from_env("REDIS_CONNECT_TIMEOUT_MS", 2_000).context("REDIS_CONNECT_TIMEOUT_MS")?;
        let sfu_connect_timeout =
            duration_from_env("SFU_CONNECT_TIMEOUT_MS", 2_000).context("SFU_CONNECT_TIMEOUT_MS")?;
        let ws_read_timeout =
            duration_from_env("WS_READ_TIMEOUT_MS", 30_000).context("WS_READ_TIMEOUT_MS")?;
        let ws_write_timeout =
            duration_from_env("WS_WRITE_TIMEOUT_MS", 10_000).context("WS_WRITE_TIMEOUT_MS")?;
        let heartbeat_interval =
            duration_from_env("HEARTBEAT_INTERVAL_MS", 15_000).context("HEARTBEAT_INTERVAL_MS")?;

        Ok(Self {
            signaling_id,
            signaling_addr,
            redis_url,
            sfu_grpc_addr,
            redis_connect_timeout,
            sfu_connect_timeout,
            ws_read_timeout,
            ws_write_timeout,
            heartbeat_interval,
        })
    }
}

fn get_env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn duration_from_env(key: &str, default_ms: u64) -> Result<Duration> {
    match env::var(key) {
        Ok(value) => {
            let ms: u64 = value.parse().with_context(|| format!("invalid integer for {key}: {value}"))?;
            Ok(Duration::from_millis(ms))
        }
        Err(_) => Ok(Duration::from_millis(default_ms)),
    }
}