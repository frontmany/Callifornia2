use std::env;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub signaling_addr: String,
    pub signaling_public_host: String,
    pub signaling_public_port: u16,
    pub redis_url: String,
    pub room_manager_grpc_addr: String,
    pub connector_token_secret: String,
    pub redis_connect_timeout: Duration,
    pub rpc_connect_timeout: Duration,
    pub ws_read_timeout: Duration,
    pub ws_write_timeout: Duration,
    pub heartbeat_interval: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let signaling_addr = get_env_or("SIGNALING_ADDR", "0.0.0.0:8080");
        let default_public_port = port_from_addr(&signaling_addr).unwrap_or(8080);
        let default_public_host =
            host_from_addr(&signaling_addr).unwrap_or_else(|| "127.0.0.1".to_owned());
        let signaling_public_host = get_env_or("SIGNALING_PUBLIC_HOST", &default_public_host);
        let signaling_public_port = u16_from_env_or("SIGNALING_PUBLIC_PORT", default_public_port)
            .context("SIGNALING_PUBLIC_PORT")?;
        let redis_url = get_env_or("REDIS_URL", "redis://redis:6379/");
        let room_manager_grpc_addr =
            get_env_or("ROOM_MANAGER_GRPC_ADDR", "http://room-manager:50061");
        let connector_token_secret =
            get_env_or("CONNECTOR_TOKEN_SECRET", "dev-connector-token-secret");

        let redis_connect_timeout = duration_from_env("REDIS_CONNECT_TIMEOUT_MS", 2_000)
            .context("REDIS_CONNECT_TIMEOUT_MS")?;
        let rpc_connect_timeout =
            duration_from_env("SFU_CONNECT_TIMEOUT_MS", 2_000).context("SFU_CONNECT_TIMEOUT_MS")?;
        let ws_read_timeout =
            duration_from_env("WS_READ_TIMEOUT_MS", 30_000).context("WS_READ_TIMEOUT_MS")?;
        let ws_write_timeout =
            duration_from_env("WS_WRITE_TIMEOUT_MS", 10_000).context("WS_WRITE_TIMEOUT_MS")?;
        let heartbeat_interval =
            duration_from_env("HEARTBEAT_INTERVAL_MS", 15_000).context("HEARTBEAT_INTERVAL_MS")?;

        Ok(Self {
            signaling_addr,
            signaling_public_host,
            signaling_public_port,
            redis_url,
            room_manager_grpc_addr,
            connector_token_secret,
            redis_connect_timeout,
            rpc_connect_timeout,
            ws_read_timeout,
            ws_write_timeout,
            heartbeat_interval,
        })
    }

    pub fn public_node_id(&self) -> String {
        format!(
            "{}:{}",
            self.signaling_public_host, self.signaling_public_port
        )
    }
}

fn get_env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn duration_from_env(key: &str, default_ms: u64) -> Result<Duration> {
    match env::var(key) {
        Ok(value) => {
            let ms: u64 = value
                .parse()
                .with_context(|| format!("invalid integer for {key}: {value}"))?;
            Ok(Duration::from_millis(ms))
        }
        Err(_) => Ok(Duration::from_millis(default_ms)),
    }
}

fn u16_from_env_or(key: &str, default: u16) -> Result<u16> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid integer for {key}: {value}")),
        Err(_) => Ok(default),
    }
}

fn port_from_addr(addr: &str) -> Option<u16> {
    addr.parse::<SocketAddr>().ok().map(|value| value.port())
}

fn host_from_addr(addr: &str) -> Option<String> {
    addr.parse::<SocketAddr>()
        .ok()
        .map(|value| value.ip().to_string())
}
