mod config;
mod proto;
mod probes;

use std::sync::Arc;

use anyhow::{Context, Result};
use control_store::storage::{self, health_ping, unix_now};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::probes::{
    heartbeat_loop, janitor_loop, purge_all_signaling_for_redis_down,
    room_manager_probe_loop, sfu_probe_loop, signaling_probe_loop,
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    init_tracing();

    let config = Arc::new(Config::from_env().context("load supervisor config")?);
    storage::set_op_timeout(config.redis_op_timeout);

    let redis = redis::Client::open(config.redis_url.as_str())
        .context("create redis client")?;

    // Wait for Redis to be reachable.
    let connect_deadline =
        std::time::Instant::now() + config.redis_connect_timeout + std::time::Duration::from_secs(5);
    loop {
        if health_ping(&redis).await.is_ok() {
            info!("Redis is reachable");
            break;
        }
        if std::time::Instant::now() > connect_deadline {
            return Err(anyhow::anyhow!("Redis not reachable within timeout"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Write first supervisor heartbeat immediately.
    let now = unix_now();
    if let Err(err) =
        storage::write_supervisor_heartbeat(&redis, Some(&config.instance_id), now).await
    {
        warn!(error = %err, "failed to write initial supervisor heartbeat");
    }

    info!(instance_id = %config.instance_id, "Supervisor started");

    // Spawn background tasks.

    let hb_redis = redis.clone();
    let hb_instance_id = config.instance_id.clone();
    let hb_interval = config.probe_interval;
    tokio::spawn(async move {
        heartbeat_loop(hb_redis, hb_instance_id, hb_interval).await;
    });

    let sig_config = config.clone();
    let sig_redis = redis.clone();
    tokio::spawn(async move {
        signaling_probe_loop(sig_config, sig_redis).await;
    });

    let rm_config = config.clone();
    let rm_redis = redis.clone();
    tokio::spawn(async move {
        room_manager_probe_loop(rm_config, rm_redis).await;
    });

    let sfu_config = config.clone();
    let sfu_redis = redis.clone();
    tokio::spawn(async move {
        sfu_probe_loop(sfu_config, sfu_redis).await;
    });

    let jan_config = config.clone();
    let jan_redis = redis.clone();
    tokio::spawn(async move {
        janitor_loop(jan_config, jan_redis).await;
    });

    // Redis health monitoring: purge signaling if Redis goes down.
    let mut redis_was_down = false;
    let mut redis_check = tokio::time::interval(config.probe_interval);
    loop {
        redis_check.tick().await;
        match health_ping(&redis).await {
            Ok(()) => {
                if redis_was_down {
                    info!("Redis recovered");
                }
                redis_was_down = false;
            }
            Err(err) => {
                if !redis_was_down {
                    error!(error = %err, "Redis is down");
                    purge_all_signaling_for_redis_down(&config).await;
                }
                redis_was_down = true;
            }
        }
    }
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
