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
    pub token_secret: String,
    pub token_ttl: Duration,
    pub session_ttl: Duration,
    pub signaling_stale_timeout: Duration,
    /// How many seconds since supervisor heartbeat before rejecting /create.
    pub supervisor_stale_sec: u64,
    pub signaling_instances: Vec<SignalingInstance>,
    /// Comma-separated list of browser origins allowed by CORS.
    pub cors_allowed_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let connector_addr = get_env_or("CONNECTOR_ADDR", "0.0.0.0:8090");
        let redis_url = get_env_or("REDIS_URL", "redis://redis:6379/");
        let redis_connect_timeout = duration_from_env("REDIS_CONNECT_TIMEOUT_MS", 2_000)
            .context("REDIS_CONNECT_TIMEOUT_MS")?;
        let redis_op_timeout =
            duration_from_env("REDIS_OP_TIMEOUT_MS", 800).context("REDIS_OP_TIMEOUT_MS")?;
        let token_secret = get_env_or("CONNECTOR_TOKEN_SECRET", "dev-connector-token-secret");
        let token_ttl = duration_from_env("CONNECTOR_TOKEN_TTL_MS", 120_000)
            .context("CONNECTOR_TOKEN_TTL_MS")?;
        let session_ttl =
            duration_from_secs_env("SESSION_TTL_SEC", 600).context("SESSION_TTL_SEC")?;
        let signaling_stale_timeout =
            duration_from_secs_env("SIGNALING_STALE_SEC", 30).context("SIGNALING_STALE_SEC")?;
        let supervisor_stale_sec = u64_env_or("SUPERVISOR_STALE_SEC", 30)?;
        let signaling_instances = parse_signaling_instances(&get_env_or(
            "SIGNALING_INSTANCES",
            "127.0.0.1:8080|ws://127.0.0.1:8080/ws",
        ))?;
        let cors_allowed_origins = parse_csv_list_env("CONNECTOR_CORS_ALLOWED_ORIGINS");

        Ok(Self {
            connector_addr,
            redis_url,
            redis_connect_timeout,
            redis_op_timeout,
            token_secret,
            token_ttl,
            session_ttl,
            signaling_stale_timeout,
            supervisor_stale_sec,
            signaling_instances,
            cors_allowed_origins,
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

fn parse_csv_list_env(key: &str) -> Vec<String> {
    env::var(key)
        .unwrap_or_default()
        .split(',')
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
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

fn u64_env_or(key: &str, default: u64) -> Result<u64> {
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
