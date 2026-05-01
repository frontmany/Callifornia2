use std::env;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct SignalingInstance {
    pub id: String,
    pub ws_url: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub connector_addr: String,
    pub redis_url: String,
    pub redis_connect_timeout: Duration,
    pub redis_op_timeout: Duration,
    pub redis_probe_interval: Duration,
    pub ws_write_timeout: Duration,
    pub token_secret: String,
    pub token_ttl: Duration,
    pub session_ttl: Duration,
    pub nick_lease_ttl: Duration,
    pub signaling_stale_timeout: Duration,
    pub signaling_instances: Vec<SignalingInstance>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let connector_addr = get_env_or("CONNECTOR_ADDR", "0.0.0.0:8090");
        let redis_url = get_env_or("REDIS_URL", "redis://redis:6379/");
        let redis_connect_timeout = duration_from_env("REDIS_CONNECT_TIMEOUT_MS", 2_000)
            .context("REDIS_CONNECT_TIMEOUT_MS")?;
        let redis_op_timeout =
            duration_from_env("REDIS_OP_TIMEOUT_MS", 800).context("REDIS_OP_TIMEOUT_MS")?;
        let redis_probe_interval = duration_from_env("REDIS_PROBE_INTERVAL_MS", 1_000)
            .context("REDIS_PROBE_INTERVAL_MS")?;
        let ws_write_timeout =
            duration_from_env("WS_WRITE_TIMEOUT_MS", 10_000).context("WS_WRITE_TIMEOUT_MS")?;
        let token_secret = get_env_or("CONNECTOR_TOKEN_SECRET", "dev-connector-token-secret");
        let token_ttl = duration_from_env("CONNECTOR_TOKEN_TTL_MS", 120_000)
            .context("CONNECTOR_TOKEN_TTL_MS")?;
        let session_ttl =
            duration_from_secs_env("SESSION_TTL_SEC", 600).context("SESSION_TTL_SEC")?;
        let nick_lease_ttl =
            duration_from_secs_env("NICK_LEASE_TTL_SEC", 30).context("NICK_LEASE_TTL_SEC")?;
        let signaling_stale_timeout = duration_from_secs_env("SIGNALING_STALE_SEC", 30)
            .context("SIGNALING_STALE_SEC")?;
        let signaling_instances = parse_signaling_instances(&get_env_or(
            "SIGNALING_INSTANCES",
            "127.0.0.1:8080|ws://127.0.0.1:8080/ws",
        ))?;

        Ok(Self {
            connector_addr,
            redis_url,
            redis_connect_timeout,
            redis_op_timeout,
            redis_probe_interval,
            ws_write_timeout,
            token_secret,
            token_ttl,
            session_ttl,
            nick_lease_ttl,
            signaling_stale_timeout,
            signaling_instances,
        })
    }

    pub fn signaling_by_id(&self, id: &str) -> Option<&SignalingInstance> {
        self.signaling_instances
            .iter()
            .find(|instance| instance.id == id)
    }
}

fn parse_signaling_instances(value: &str) -> Result<Vec<SignalingInstance>> {
    value
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let (id, ws_url) = entry
                .split_once('|')
                .context("SIGNALING_INSTANCES entries must be id|ws_url")?;
            Ok(SignalingInstance {
                id: id.to_owned(),
                ws_url: ws_url.to_owned(),
            })
        })
        .collect()
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
