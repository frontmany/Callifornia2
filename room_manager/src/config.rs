use std::env;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::models::SfuCandidate;

#[derive(Debug, Clone)]
pub struct Config {
    pub grpc_addr: String,
    pub redis_url: String,
    pub redis_connect_timeout: Duration,
    pub sfu_connect_timeout: Duration,
    pub health_interval: Duration,
    pub scale_down_idle_timeout: Duration,
    pub max_waiting_requests: usize,
    pub sfu_candidates: Vec<SfuCandidate>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            grpc_addr: env_or("ROOM_MANAGER_ADDR", "0.0.0.0:50061"),
            redis_url: env_or("REDIS_URL", "redis://redis:6379/"),
            redis_connect_timeout: ms_env_or("REDIS_CONNECT_TIMEOUT_MS", 2_000)?,
            sfu_connect_timeout: ms_env_or("SFU_CONNECT_TIMEOUT_MS", 2_000)?,
            health_interval: ms_env_or("ROOM_MANAGER_HEALTH_INTERVAL_MS", 5_000)?,
            scale_down_idle_timeout: ms_env_or("ROOM_MANAGER_SCALE_DOWN_IDLE_MS", 300_000)?,
            max_waiting_requests: usize_env_or("ROOM_MANAGER_MAX_WAITING_REQUESTS", 10_000)?,
            sfu_candidates: parse_candidates(&env_or("ROOM_MANAGER_SFU_CANDIDATES", ""))?,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn ms_env_or(key: &str, default_ms: u64) -> Result<Duration> {
    Ok(Duration::from_millis(u64_env_or(key, default_ms)?))
}

fn usize_env_or(key: &str, default: usize) -> Result<usize> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid integer for {key}: {value}")),
        Err(_) => Ok(default),
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

fn parse_candidates(value: &str) -> Result<Vec<SfuCandidate>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    value
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let mut parts = entry.split('|');
            let instance_id = parts
                .next()
                .filter(|value| !value.is_empty())
                .context("candidate instance_id is required")?;
            let grpc_addr = parts
                .next()
                .filter(|value| !value.is_empty())
                .context("candidate grpc_addr is required")?;
            let max_clients = parts
                .next()
                .context("candidate max_clients is required")?
                .parse()
                .context("candidate max_clients must be integer")?;

            Ok(SfuCandidate {
                instance_id: instance_id.to_owned(),
                grpc_addr: grpc_addr.to_owned(),
                max_clients,
            })
        })
        .collect()
}
