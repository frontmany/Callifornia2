use std::sync::Arc;
use std::time::Duration;

use control_store::storage::{
    alive_signaling_nodes, decrement_sfu_room_load, delete_room_assignment,
    delete_signaling_room_members, delete_signaling_session, get_room_assignment,
    list_sfu_instances, remove_signaling_load, remove_signaling_node,
    remove_signaling_node_heartbeats_below, scan_room_binding_ids, scan_signaling_session_ids,
    signaling_load_members, unix_now, update_sfu_health, write_signaling_node_seen,
    write_supervisor_heartbeat,
};
use tonic::transport::{Channel, Endpoint};
use tracing::{error, warn};

use crate::config::{Config, SignalingAdminInstance};
use crate::proto::admin_pb::signaling_admin_service_client::SignalingAdminServiceClient;
use crate::proto::admin_pb::{CloseRoomsBySfuRequest, PingRequest, PurgeRequest};
use crate::proto::sfu_pb::sfu_service_client::SfuServiceClient;
use crate::proto::sfu_pb::PingRequest as SfuPingRequest;

// ── Background loops ──────────────────────────────────────────────────────────

pub async fn heartbeat_loop(redis: redis::Client, instance_id: String, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let now = unix_now();
        if let Err(err) = write_supervisor_heartbeat(&redis, Some(&instance_id), now).await {
            warn!(error = %err, "failed to write supervisor heartbeat");
        }
    }
}

pub async fn signaling_probe_loop(config: Arc<Config>, redis: redis::Client) {
    let mut ticker = tokio::time::interval(config.probe_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        for instance in &config.signaling_admin_instances {
            probe_signaling_instance(&config, &redis, instance).await;
        }
    }
}

async fn probe_signaling_instance(
    config: &Config,
    redis: &redis::Client,
    instance: &SignalingAdminInstance,
) {
    let result = tokio::time::timeout(
        config.probe_timeout,
        ping_signaling_admin(&instance.grpc_addr, config.rpc_connect_timeout),
    )
    .await;

    match result {
        Ok(Ok(_resp)) => {
            let now = unix_now();
            if let Err(err) = write_signaling_node_seen(redis, &instance.node_id, now).await {
                warn!(error = %err, node = %instance.node_id, "failed to write signaling node seen");
            }
        }
        Ok(Err(err)) => {
            warn!(error = %err, node = %instance.node_id, "signaling admin probe failed");
            check_and_purge_dead_signaling(config, redis, instance).await;
        }
        Err(_) => {
            warn!(node = %instance.node_id, "signaling admin probe timed out");
            check_and_purge_dead_signaling(config, redis, instance).await;
        }
    }
}

async fn check_and_purge_dead_signaling(
    config: &Config,
    redis: &redis::Client,
    instance: &SignalingAdminInstance,
) {
    let alive = match alive_signaling_nodes(redis, config.signaling_stale_timeout).await {
        Ok(nodes) => nodes,
        Err(err) => {
            warn!(error = %err, "failed to read alive signaling nodes");
            return;
        }
    };
    if alive.contains(&instance.node_id) {
        // Node still within stale window; don't reclaim yet.
        return;
    }
    // Node is past stale threshold: reclaim all its rooms.
    warn!(node = %instance.node_id, "signaling node stale; reclaiming");
    if let Err(err) = remove_signaling_node(redis, &instance.node_id).await {
        warn!(error = %err, node = %instance.node_id, "failed to remove stale signaling node");
    }
}

pub async fn sfu_probe_loop(config: Arc<Config>, redis: redis::Client) {
    let mut ticker = tokio::time::interval(config.probe_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let instances = match list_sfu_instances(&redis).await {
            Ok(instances) => instances,
            Err(err) => {
                warn!(error = %err, "failed to list SFU instances");
                continue;
            }
        };

        let now = unix_now();
        for instance in instances {
            let alive = ping_sfu(&instance.grpc_addr, config.sfu_connect_timeout).await;
            if let Err(err) = update_sfu_health(&redis, &instance.instance_id, alive, now).await {
                warn!(error = %err, instance_id = %instance.instance_id, "failed to update SFU health");
                continue;
            }

            if !alive {
                handle_dead_sfu(&config, &redis, &instance.grpc_addr).await;
            }
        }
    }
}

async fn handle_dead_sfu(config: &Config, redis: &redis::Client, sfu_grpc_addr: &str) {
    // For each room binding pointing to this SFU, ask the owning signaling
    // node to close those rooms (CloseRoomsBySfu), then clean Redis.
    let binding_ids = match scan_room_binding_ids(redis).await {
        Ok(ids) => ids,
        Err(err) => {
            warn!(error = %err, "failed to scan room bindings for dead SFU");
            return;
        }
    };

    for room_id in &binding_ids {
        let binding = match get_room_assignment(redis, room_id).await {
            Ok(Some(b)) => b,
            Ok(None) => continue,
            Err(err) => {
                warn!(error = %err, room_id = %room_id, "failed to get room assignment");
                continue;
            }
        };

        if binding.sfu_grpc_addr.as_deref() != Some(sfu_grpc_addr) {
            continue;
        }

        // Ask the owning signaling instance to close rooms on the dead SFU.
        if !binding.owner_id.is_empty() {
            if let Some(inst) = config
                .signaling_admin_instances
                .iter()
                .find(|i| i.node_id == binding.owner_id)
            {
                let reason = "sfu_down";
                if let Err(err) = close_rooms_by_sfu_on_signaling(
                    &inst.grpc_addr,
                    config.rpc_connect_timeout,
                    sfu_grpc_addr,
                    reason,
                )
                .await
                {
                    warn!(
                        error = %err,
                        node = %inst.node_id,
                        sfu_grpc_addr = %sfu_grpc_addr,
                        "failed to call CloseRoomsBySfu on signaling"
                    );
                }
            }
        }

        // Clean Redis regardless of whether signaling was reachable.
        if let Some(sfu_instance_id) = binding.sfu_instance_id.as_deref() {
            if let Err(err) = decrement_sfu_room_load(redis, sfu_instance_id).await {
                warn!(error = %err, room_id = %room_id, "janitor: failed to decrement sfu load");
            }
        }
        if let Err(err) = delete_room_assignment(redis, room_id).await {
            warn!(error = %err, room_id = %room_id, "failed to delete room assignment for dead SFU");
        }
        if let Err(err) = delete_signaling_room_members(redis, room_id).await {
            warn!(error = %err, room_id = %room_id, "failed to delete room members for dead SFU");
        }
    }
}

// ── Janitor ───────────────────────────────────────────────────────────────────

pub async fn janitor_loop(config: Arc<Config>, redis: redis::Client) {
    let interval = config.janitor_interval;
    loop {
        if let Err(err) = run_janitor_iteration(&config, &redis).await {
            error!(error = %err, "janitor iteration failed");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_janitor_iteration(config: &Config, redis: &redis::Client) -> anyhow::Result<()> {
    let stale_threshold = config.signaling_stale_timeout;
    let alive_nodes = alive_signaling_nodes(redis, stale_threshold).await?;
    let alive_set: std::collections::HashSet<String> = alive_nodes.into_iter().collect();
    let now = unix_now();
    let cutoff = now - stale_threshold.as_secs() as i64;

    // Reclaim rooms owned by stale signaling nodes.
    let binding_ids = scan_room_binding_ids(redis).await?;
    for room_id in &binding_ids {
        let Some(binding) = get_room_assignment(redis, room_id).await? else {
            continue;
        };
        if binding.owner_id.is_empty() || alive_set.contains(&binding.owner_id) {
            continue;
        }
        warn!(
            room_id = %room_id,
            owner = %binding.owner_id,
            "janitor: reclaiming room from stale signaling node"
        );
        if let Some(sfu_instance_id) = binding.sfu_instance_id.as_deref() {
            if let Err(err) = decrement_sfu_room_load(redis, sfu_instance_id).await {
                warn!(error = %err, room_id = %room_id, "janitor: failed to decrement sfu load");
            }
        }
        if let Err(err) = delete_room_assignment(redis, room_id).await {
            warn!(error = %err, room_id = %room_id, "janitor: failed to delete room binding");
        }
        if let Err(err) = delete_signaling_room_members(redis, room_id).await {
            warn!(error = %err, room_id = %room_id, "janitor: failed to delete room members");
        }
    }

    // Remove orphan signaling room-member sets (no binding).
    let binding_set: std::collections::HashSet<String> = binding_ids.into_iter().collect();
    let signaling_room_ids = control_store::storage::scan_signaling_room_ids(redis).await?;
    for room_id in &signaling_room_ids {
        if !binding_set.contains(room_id) {
            warn!(room_id = %room_id, "janitor: deleting orphan signaling room members");
            if let Err(err) = delete_signaling_room_members(redis, room_id).await {
                warn!(error = %err, room_id = %room_id, "janitor: failed to delete orphan room members");
            }
        }
    }

    // Remove stale signaling load entries.
    let load_members = signaling_load_members(redis).await?;
    for node_id in load_members {
        if !alive_set.contains(&node_id) {
            warn!(node = %node_id, "janitor: removing stale signaling instance load");
            if let Err(err) = remove_signaling_load(redis, &node_id).await {
                warn!(error = %err, node = %node_id, "janitor: failed to remove stale load");
            }
        }
    }

    // Delete sessions owned by stale signaling nodes.
    let session_ids = scan_signaling_session_ids(redis).await?;
    for session_id in session_ids {
        let owner = match control_store::storage::get_session_owner(redis, &session_id).await? {
            Some(owner) => owner,
            None => continue,
        };
        if alive_set.contains(&owner) {
            continue;
        }
        warn!(
            session_id = %session_id,
            owner = %owner,
            "janitor: deleting session of stale signaling node"
        );
        if let Err(err) = delete_signaling_session(redis, &session_id).await {
            warn!(error = %err, session_id = %session_id, "janitor: failed to delete stale session");
        }
    }

    // Compact stale heartbeat scores.
    if let Err(err) = remove_signaling_node_heartbeats_below(redis, cutoff).await {
        warn!(error = %err, "janitor: failed to clean stale signaling node heartbeats");
    }

    Ok(())
}

// ── gRPC helpers ──────────────────────────────────────────────────────────────

async fn grpc_channel(addr: &str, connect_timeout: Duration) -> anyhow::Result<Channel> {
    let endpoint = Endpoint::from_shared(addr.to_owned())
        .map_err(|e| anyhow::anyhow!("invalid gRPC addr {addr}: {e}"))?
        .connect_timeout(connect_timeout)
        .tcp_nodelay(true)
        .http2_keep_alive_interval(Duration::from_secs(20))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true);
    let channel = endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("gRPC connect to {addr} failed: {e}"))?;
    Ok(channel)
}

pub async fn ping_signaling_admin(addr: &str, connect_timeout: Duration) -> anyhow::Result<()> {
    let channel = grpc_channel(addr, connect_timeout).await?;
    SignalingAdminServiceClient::new(channel)
        .ping(PingRequest {})
        .await
        .map_err(|s| anyhow::anyhow!("admin Ping failed: {s}"))?;
    Ok(())
}

async fn close_rooms_by_sfu_on_signaling(
    addr: &str,
    connect_timeout: Duration,
    sfu_grpc_addr: &str,
    reason: &str,
) -> anyhow::Result<()> {
    let channel = grpc_channel(addr, connect_timeout).await?;
    SignalingAdminServiceClient::new(channel)
        .close_rooms_by_sfu(CloseRoomsBySfuRequest {
            sfu_grpc_addr: sfu_grpc_addr.to_owned(),
            reason: reason.to_owned(),
        })
        .await
        .map_err(|s| anyhow::anyhow!("CloseRoomsBySfu failed: {s}"))?;
    Ok(())
}

pub async fn purge_signaling(
    addr: &str,
    connect_timeout: Duration,
    reason: &str,
) -> anyhow::Result<()> {
    let channel = grpc_channel(addr, connect_timeout).await?;
    SignalingAdminServiceClient::new(channel)
        .purge(PurgeRequest {
            reason: reason.to_owned(),
        })
        .await
        .map_err(|s| anyhow::anyhow!("Purge failed: {s}"))?;
    Ok(())
}

pub async fn ping_sfu(grpc_addr: &str, connect_timeout: Duration) -> bool {
    let endpoint = match Endpoint::from_shared(grpc_addr.to_owned()) {
        Ok(ep) => ep.connect_timeout(connect_timeout).timeout(connect_timeout),
        Err(_) => return false,
    };
    let channel = match endpoint.connect().await {
        Ok(ch) => ch,
        Err(_) => return false,
    };
    matches!(
        tokio::time::timeout(
            connect_timeout,
            SfuServiceClient::new(channel).ping(SfuPingRequest {})
        )
        .await,
        Ok(Ok(_))
    )
}

/// Called when Redis is detected down: purge all reachable signaling nodes.
pub async fn purge_all_signaling_for_redis_down(config: &Config) {
    warn!("Redis is down; purging all reachable signaling nodes");
    for instance in &config.signaling_admin_instances {
        if let Err(err) = tokio::time::timeout(
            config.probe_timeout,
            purge_signaling(&instance.grpc_addr, config.rpc_connect_timeout, "redis"),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("purge timeout")))
        {
            warn!(error = %err, node = %instance.node_id, "failed to purge signaling for redis down");
        }
    }
}
