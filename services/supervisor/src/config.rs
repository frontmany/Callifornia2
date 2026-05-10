use std::env;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct SignalingAdminInstance {
    /// Node id used in signaling:nodes ("host:port").
    pub node_id: String,
    /// gRPC address of the admin server, e.g. "http://127.0.0.1:50071".
    pub grpc_addr: String,
}

#[derive(Debug, Clone)]
pub struct SfuInventoryInstance {
    pub instance_id: String,
    pub grpc_addr: String,
    pub max_rooms: u32,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub redis_url: String,
    pub redis_connect_timeout: Duration,
    pub redis_op_timeout: Duration,

    pub rpc_connect_timeout: Duration,

    /// List of signaling admin endpoints to probe.
    pub signaling_admin_instances: Vec<SignalingAdminInstance>,
    /// List of SFU endpoints to seed into Redis and probe.
    pub sfu_instances: Vec<SfuInventoryInstance>,

    /// How often each probe loop ticks.
    pub probe_interval: Duration,
    /// Timeout for individual probe RPCs.
    pub probe_timeout: Duration,

    /// How long a signaling node may be missing from probes before considered stale.
    pub signaling_stale_timeout: Duration,

    /// Janitor / reconciliation interval.
    pub janitor_interval: Duration,

    /// Timeout for individual SFU Ping RPCs.
    pub sfu_connect_timeout: Duration,

    /// Optional instance id written to supervisor:heartbeat (defaults to a UUID).
    pub instance_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            redis_url: env_or("REDIS_URL", "redis://redis:6379/"),
            redis_connect_timeout: ms_env_or("REDIS_CONNECT_TIMEOUT_MS", 2_000)?,
            redis_op_timeout: ms_env_or("REDIS_OP_TIMEOUT_MS", 800)?,
            rpc_connect_timeout: ms_env_or("RPC_CONNECT_TIMEOUT_MS", 2_000)?,
            signaling_admin_instances: parse_signaling_admin_instances(&env_or(
                "SIGNALING_ADMIN_INSTANCES",
                "",
            ))?,
            sfu_instances: parse_sfu_instances(&env_or("SUPERVISOR_SFU_INSTANCES", ""))?,
            probe_interval: ms_env_or("SUPERVISOR_PROBE_INTERVAL_MS", 2_000)?,
            probe_timeout: ms_env_or("SUPERVISOR_PROBE_TIMEOUT_MS", 1_000)?,
            signaling_stale_timeout: secs_env_or("SIGNALING_STALE_SEC", 30)?,
            janitor_interval: secs_env_or("JANITOR_INTERVAL_SEC", 5)?,
            sfu_connect_timeout: ms_env_or("SFU_CONNECT_TIMEOUT_MS", 2_000)?,
            instance_id: env_or("SUPERVISOR_INSTANCE_ID", &uuid::Uuid::new_v4().to_string()),
        })
    }
}

/// Format: "node_id|grpc_addr,node_id|grpc_addr,..."
/// Example: "127.0.0.1:8080|http://127.0.0.1:50071"
fn parse_signaling_admin_instances(value: &str) -> Result<Vec<SignalingAdminInstance>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .filter(|e| !e.trim().is_empty())
        .map(|entry| {
            let (node_id, grpc_addr) = entry
                .split_once('|')
                .context("SIGNALING_ADMIN_INSTANCES must be node_id|grpc_addr pairs")?;
            Ok(SignalingAdminInstance {
                node_id: node_id.trim().to_owned(),
                grpc_addr: grpc_addr.trim().to_owned(),
            })
        })
        .collect()
}

/// Format: "instance_id|grpc_addr|max_rooms;instance_id|grpc_addr|max_rooms;..."
/// Example: "sfu-1|http://127.0.0.1:50051|200"
fn parse_sfu_instances(value: &str) -> Result<Vec<SfuInventoryInstance>> {
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
                .context("SUPERVISOR_SFU_INSTANCES entry requires instance_id")?;
            let grpc_addr = parts
                .next()
                .filter(|value| !value.is_empty())
                .context("SUPERVISOR_SFU_INSTANCES entry requires grpc_addr")?;
            let max_rooms = parts
                .next()
                .context("SUPERVISOR_SFU_INSTANCES entry requires max_rooms")?
                .parse()
                .context("SUPERVISOR_SFU_INSTANCES max_rooms must be integer")?;
            Ok(SfuInventoryInstance {
                instance_id: instance_id.trim().to_owned(),
                grpc_addr: grpc_addr.trim().to_owned(),
                max_rooms,
            })
        })
        .collect()
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn ms_env_or(key: &str, default_ms: u64) -> Result<Duration> {
    Ok(Duration::from_millis(u64_env_or(key, default_ms)?))
}

fn secs_env_or(key: &str, default_secs: u64) -> Result<Duration> {
    Ok(Duration::from_secs(u64_env_or(key, default_secs)?))
}

fn u64_env_or(key: &str, default: u64) -> Result<u64> {
    match env::var(key) {
        Ok(v) => v
            .parse()
            .with_context(|| format!("invalid integer for {key}: {v}")),
        Err(_) => Ok(default),
    }
}
