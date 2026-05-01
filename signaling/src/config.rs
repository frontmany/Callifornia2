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
    pub redis_op_timeout: Duration,
    pub room_manager_probe_interval: Duration,
    pub room_manager_probe_timeout: Duration,
    pub sfu_rpc_timeout: Duration,
    pub sfu_backoff_min: Duration,
    pub sfu_backoff_max: Duration,
    pub peer_outbound_capacity: usize,
    pub signaling_heartbeat: Duration,
    pub session_lock_ttl: Duration,
    pub nick_lease_ttl: Duration,
    pub nick_lease_renew: Duration,
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

        let redis_op_timeout =
            duration_from_env("REDIS_OP_TIMEOUT_MS", 800).context("REDIS_OP_TIMEOUT_MS")?;
        let room_manager_probe_interval =
            duration_from_env("ROOM_MANAGER_PROBE_INTERVAL_MS", 1_000)
                .context("ROOM_MANAGER_PROBE_INTERVAL_MS")?;
        let room_manager_probe_timeout = duration_from_env("ROOM_MANAGER_PROBE_TIMEOUT_MS", 500)
            .context("ROOM_MANAGER_PROBE_TIMEOUT_MS")?;
        let sfu_rpc_timeout =
            duration_from_env("SFU_RPC_TIMEOUT_MS", 5_000).context("SFU_RPC_TIMEOUT_MS")?;
        let sfu_backoff_min =
            duration_from_env("SFU_BACKOFF_MIN_MS", 1_000).context("SFU_BACKOFF_MIN_MS")?;
        let sfu_backoff_max =
            duration_from_env("SFU_BACKOFF_MAX_MS", 30_000).context("SFU_BACKOFF_MAX_MS")?;
        let peer_outbound_capacity = usize_from_env_or("PEER_OUTBOUND_CHANNEL_CAPACITY", 256)
            .context("PEER_OUTBOUND_CHANNEL_CAPACITY")?;
        let signaling_heartbeat =
            duration_from_secs_env("SIGNALING_HEARTBEAT_SEC", 5).context("SIGNALING_HEARTBEAT_SEC")?;
        let session_lock_ttl =
            duration_from_env("SESSION_LOCK_TTL_MS", 5_000).context("SESSION_LOCK_TTL_MS")?;
        let nick_lease_ttl =
            duration_from_secs_env("NICK_LEASE_TTL_SEC", 30).context("NICK_LEASE_TTL_SEC")?;
        let nick_lease_renew =
            duration_from_secs_env("NICK_LEASE_RENEW_SEC", 10).context("NICK_LEASE_RENEW_SEC")?;

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
            redis_op_timeout,
            room_manager_probe_interval,
            room_manager_probe_timeout,
            sfu_rpc_timeout,
            sfu_backoff_min,
            sfu_backoff_max,
            peer_outbound_capacity,
            signaling_heartbeat,
            session_lock_ttl,
            nick_lease_ttl,
            nick_lease_renew,
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

fn usize_from_env_or(key: &str, default: usize) -> Result<usize> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid integer for {key}: {value}")),
        Err(_) => Ok(default),
    }
}

fn duration_from_secs_env(key: &str, default_secs: u64) -> Result<Duration> {
    match env::var(key) {
        Ok(value) => {
            let secs: u64 = value
                .parse()
                .with_context(|| format!("invalid integer for {key}: {value}"))?;
            Ok(Duration::from_secs(secs))
        }
        Err(_) => Ok(Duration::from_secs(default_secs)),
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
