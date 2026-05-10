use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use redis::AsyncCommands;
use tokio::time::timeout;

use crate::keys::*;
use crate::models::{RoomBinding, SfuCandidate, SfuInstance, SupervisorStatus};

static OP_TIMEOUT: OnceLock<Duration> = OnceLock::new();

pub fn set_op_timeout(value: Duration) {
    let _ = OP_TIMEOUT.set(value);
}

fn op_timeout() -> Duration {
    OP_TIMEOUT.get().copied().unwrap_or(Duration::from_secs(2))
}

async fn with_timeout<T, F>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match timeout(op_timeout(), fut).await {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!("redis operation timed out")),
    }
}

// ── Supervisor heartbeat ──────────────────────────────────────────────────────

pub async fn write_supervisor_heartbeat(
    redis: &redis::Client,
    instance_id: Option<&str>,
    now_unix: i64,
) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: () = redis::pipe()
            .atomic()
            .hset(SUPERVISOR_HEARTBEAT_KEY, "last_seen_unix", now_unix)
            .ignore()
            .hset(
                SUPERVISOR_HEARTBEAT_KEY,
                "instance_id",
                instance_id.unwrap_or(""),
            )
            .ignore()
            .query_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

pub async fn read_supervisor_status(redis: &redis::Client) -> Result<Option<SupervisorStatus>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(SUPERVISOR_HEARTBEAT_KEY).await?;
        if values.is_empty() {
            return Ok(None);
        }
        let last_seen_unix = values
            .get("last_seen_unix")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let instance_id = values.get("instance_id").filter(|v| !v.is_empty()).cloned();
        Ok(Some(SupervisorStatus {
            last_seen_unix,
            instance_id,
        }))
    })
    .await
}

// ── Signaling node status (written by supervisor) ─────────────────────────────
// Uses the existing ZSET signaling:nodes for backward-compat read by connector.
// Supervisor writes scores (last_seen_unix); on explicit kill sets score to 0 then ZREMs.

pub async fn write_signaling_node_seen(
    redis: &redis::Client,
    node_id: &str,
    now_unix: i64,
) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.zadd(SIGNALING_NODES_KEY, node_id, now_unix).await?;
        Ok(())
    })
    .await
}

pub async fn remove_signaling_node(redis: &redis::Client, node_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.zrem(SIGNALING_NODES_KEY, node_id).await?;
        Ok(())
    })
    .await
}

pub async fn alive_signaling_nodes(
    redis: &redis::Client,
    stale_threshold: Duration,
) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let now = unix_now();
        let cutoff = now - stale_threshold.as_secs() as i64;
        let nodes: Vec<String> = conn
            .zrangebyscore(SIGNALING_NODES_KEY, cutoff, "+inf")
            .await?;
        Ok(nodes)
    })
    .await
}

pub async fn remove_signaling_node_heartbeats_below(
    redis: &redis::Client,
    cutoff_unix: i64,
) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = redis::cmd("ZREMRANGEBYSCORE")
            .arg(SIGNALING_NODES_KEY)
            .arg("-inf")
            .arg(cutoff_unix)
            .query_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

// ── Signaling load cleanup ────────────────────────────────────────────────────

pub async fn signaling_load_members(redis: &redis::Client) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let entries: Vec<String> = conn.zrange(SIGNALING_INSTANCE_LOAD_KEY, 0, -1).await?;
        Ok(entries)
    })
    .await
}

pub async fn remove_signaling_load(redis: &redis::Client, node_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.zrem(SIGNALING_INSTANCE_LOAD_KEY, node_id).await?;
        Ok(())
    })
    .await
}

// ── Session cleanup ───────────────────────────────────────────────────────────

pub async fn scan_signaling_session_ids(redis: &redis::Client) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let mut cursor: u64 = 0;
        let mut results = Vec::new();
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{SIGNALING_SESSION_PREFIX}*"))
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await?;
            for key in keys {
                if key.ends_with(":lock") {
                    continue;
                }
                if let Some(rest) = key.strip_prefix(SIGNALING_SESSION_PREFIX) {
                    results.push(rest.to_owned());
                }
            }
            if next == 0 {
                break;
            }
            cursor = next;
        }
        Ok(results)
    })
    .await
}

pub async fn get_session_owner(redis: &redis::Client, session_id: &str) -> Result<Option<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let owner: Option<String> = conn
            .hget(session_key(session_id), "signaling_owner_id")
            .await?;
        Ok(owner.filter(|v| !v.is_empty()))
    })
    .await
}

pub async fn delete_signaling_session(redis: &redis::Client, session_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.del(session_key(session_id)).await?;
        Ok(())
    })
    .await
}

/// Delete the nick lease for a nickname if it is owned by a specific session.
pub async fn delete_nick_lease(
    redis: &redis::Client,
    nickname: &str,
    session_id: &str,
) -> Result<()> {
    const COMPARE_AND_DEL: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
else
    return 0
end
"#;
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: i64 = redis::Script::new(COMPARE_AND_DEL)
            .key(nick_key(nickname))
            .arg(session_id)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

// ── Room member cleanup ───────────────────────────────────────────────────────

pub async fn scan_signaling_room_ids(redis: &redis::Client) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let mut cursor: u64 = 0;
        let mut results = Vec::new();
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{SIGNALING_ROOM_PREFIX}*:members"))
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await?;
            for key in keys {
                if let Some(rest) = key.strip_prefix(SIGNALING_ROOM_PREFIX) {
                    if let Some(room_id) = rest.strip_suffix(":members") {
                        results.push(room_id.to_owned());
                    }
                }
            }
            if next == 0 {
                break;
            }
            cursor = next;
        }
        Ok(results)
    })
    .await
}

pub async fn delete_signaling_room_members(redis: &redis::Client, room_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.del(room_members_key(room_id)).await?;
        Ok(())
    })
    .await
}

/// List current room members.
pub async fn get_room_members(redis: &redis::Client, room_id: &str) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let members: Vec<String> = conn.smembers(room_members_key(room_id)).await?;
        Ok(members)
    })
    .await
}

// ── Room binding cleanup ──────────────────────────────────────────────────────

pub async fn scan_room_binding_ids(redis: &redis::Client) -> Result<Vec<String>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let mut cursor: u64 = 0;
        let mut results = Vec::new();
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{ROOM_BINDING_PREFIX}*:binding"))
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await?;
            for key in keys {
                if let Some(rest) = key.strip_prefix(ROOM_BINDING_PREFIX) {
                    if let Some(room_id) = rest.strip_suffix(":binding") {
                        results.push(room_id.to_owned());
                    }
                }
            }
            if next == 0 {
                break;
            }
            cursor = next;
        }
        Ok(results)
    })
    .await
}

pub async fn get_room_assignment(
    redis: &redis::Client,
    room_id: &str,
) -> Result<Option<RoomBinding>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(room_binding_key(room_id)).await?;
        if values.is_empty() {
            return Ok(None);
        }
        let owner_host = values.get("owner_host").cloned().unwrap_or_default();
        let owner_port = values
            .get("owner_port")
            .and_then(|v| v.parse().ok())
            .unwrap_or_default();
        Ok(Some(RoomBinding {
            room_id: room_id.to_owned(),
            owner_id: values
                .get("signaling_owner_id")
                .cloned()
                .unwrap_or_default(),
            owner_host,
            owner_port,
            state: values
                .get("room_state")
                .cloned()
                .unwrap_or_else(|| "active".to_owned()),
            sfu_instance_id: values
                .get("sfu_instance_id")
                .cloned()
                .filter(|v| !v.is_empty()),
            sfu_grpc_addr: values
                .get("sfu_grpc_addr")
                .cloned()
                .filter(|v| !v.is_empty()),
        }))
    })
    .await
}

pub async fn delete_room_assignment(redis: &redis::Client, room_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: () = redis::pipe()
            .atomic()
            .del(room_binding_key(room_id))
            .ignore()
            .query_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

/// Update session room field to None during cleanup.
pub async fn session_clear_room(redis: &redis::Client, session_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.hset(session_key(session_id), "room_id", "").await?;
        Ok(())
    })
    .await
}

// ── SFU instance helpers ──────────────────────────────────────────────────────

pub async fn list_sfu_instances(redis: &redis::Client) -> Result<Vec<SfuInstance>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(SFU_INSTANCES_KEY).await?;
        values
            .into_values()
            .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
            .collect()
    })
    .await
}

pub async fn load_sfu_room_loads(redis: &redis::Client) -> Result<HashMap<String, f64>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let entries: Vec<(String, f64)> = conn.zrange_withscores(SFU_ROOM_LOAD_KEY, 0, -1).await?;
        Ok(entries.into_iter().collect())
    })
    .await
}

pub async fn get_sfu_instance(
    redis: &redis::Client,
    instance_id: &str,
) -> Result<Option<SfuInstance>> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let payload: Option<String> = conn.hget(SFU_INSTANCES_KEY, instance_id).await?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
            .transpose()
    })
    .await
}

pub async fn save_sfu_instance(redis: &redis::Client, instance: &SfuInstance) -> Result<()> {
    let payload = serde_json::to_string(instance)?;
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn
            .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
            .await?;
        Ok(())
    })
    .await
}

pub async fn seed_sfu_inventory(redis: &redis::Client, inventory: &[SfuCandidate]) -> Result<()> {
    let now = unix_now();
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        for candidate in inventory {
            let existing: Option<String> =
                conn.hget(SFU_INSTANCES_KEY, &candidate.instance_id).await?;
            let instance = match existing {
                Some(payload) => {
                    let mut instance: SfuInstance = serde_json::from_str(&payload)?;
                    instance.grpc_addr = candidate.grpc_addr.clone();
                    instance.max_rooms = candidate.max_rooms;
                    instance
                }
                None => SfuInstance {
                    instance_id: candidate.instance_id.clone(),
                    grpc_addr: candidate.grpc_addr.clone(),
                    max_rooms: candidate.max_rooms,
                    alive: false,
                    state: "unknown".to_owned(),
                    last_ping_unix: 0,
                    idle_since_unix: now,
                },
            };
            let payload = serde_json::to_string(&instance)?;
            let _: usize = conn
                .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
                .await?;
            let existing_load: Option<f64> = conn
                .zscore(SFU_ROOM_LOAD_KEY, &instance.instance_id)
                .await?;
            if existing_load.is_none() {
                let _: usize = conn
                    .zadd(SFU_ROOM_LOAD_KEY, &instance.instance_id, 0)
                    .await?;
            }
        }
        Ok(())
    })
    .await
}

pub async fn update_sfu_health(
    redis: &redis::Client,
    instance_id: &str,
    alive: bool,
    now: i64,
) -> Result<()> {
    let Some(mut instance) = get_sfu_instance(redis, instance_id).await? else {
        return Ok(());
    };
    instance.alive = alive;
    if alive {
        instance.last_ping_unix = now;
        instance.state = "ready".to_owned();
    }
    save_sfu_instance(redis, &instance).await
}

pub async fn update_idle_since(
    redis: &redis::Client,
    instance_id: &str,
    idle_since_unix: i64,
) -> Result<()> {
    let mut instance = get_sfu_instance(redis, instance_id)
        .await?
        .context("missing sfu instance for idle update")?;
    instance.idle_since_unix = idle_since_unix;
    save_sfu_instance(redis, &instance).await
}

pub async fn delete_sfu_instance(redis: &redis::Client, instance_id: &str) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: () = redis::pipe()
            .atomic()
            .hdel(SFU_INSTANCES_KEY, instance_id)
            .ignore()
            .zrem(SFU_ROOM_LOAD_KEY, instance_id)
            .ignore()
            .query_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

pub async fn decrement_sfu_room_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let next = with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let current: Option<f64> = conn.zscore(SFU_ROOM_LOAD_KEY, instance_id).await?;
        let current = current.unwrap_or(0.0);
        let next = if current <= 1.0 { 0.0 } else { current - 1.0 };
        let _: usize = conn.zadd(SFU_ROOM_LOAD_KEY, instance_id, next).await?;
        Ok(next)
    })
    .await?;
    if next == 0.0 {
        update_idle_since(redis, instance_id, unix_now()).await?;
    }
    Ok(())
}

pub async fn select_least_loaded_instance(redis: &redis::Client) -> Result<Option<SfuInstance>> {
    let instances = list_sfu_instances(redis).await?;
    let room_loads = load_sfu_room_loads(redis).await?;
    let now = unix_now();

    let selected = instances
        .into_iter()
        .filter(|inst| {
            inst.alive
                && now - inst.last_ping_unix <= 30
                && inst.state == "ready"
                && room_loads
                    .get(&inst.instance_id)
                    .copied()
                    .unwrap_or_default()
                    < f64::from(inst.max_rooms)
        })
        .min_by(|a, b| {
            let al = room_loads.get(&a.instance_id).copied().unwrap_or_default();
            let bl = room_loads.get(&b.instance_id).copied().unwrap_or_default();
            al.partial_cmp(&bl).unwrap_or(std::cmp::Ordering::Equal)
        });

    Ok(selected)
}

pub async fn assign_room_to_instance(
    redis: &redis::Client,
    room_id: &str,
    signaling_owner_id: &str,
    signaling_owner_host: &str,
    signaling_owner_port: u16,
    instance: &SfuInstance,
) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let meta_key = room_binding_key(room_id);
        let _: () = redis::pipe()
            .atomic()
            .hset(&meta_key, "owner_host", signaling_owner_host)
            .hset(&meta_key, "owner_port", signaling_owner_port)
            .hset(&meta_key, "signaling_owner_id", signaling_owner_id)
            .hset(&meta_key, "room_state", "active")
            .hset(&meta_key, "sfu_instance_id", &instance.instance_id)
            .hset(&meta_key, "sfu_grpc_addr", &instance.grpc_addr)
            .ignore()
            .query_async(&mut conn)
            .await?;
        Ok(())
    })
    .await?;

    // Increment room load
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: f64 = conn
            .zincr(SFU_ROOM_LOAD_KEY, &instance.instance_id, 1)
            .await?;
        Ok(())
    })
    .await?;

    update_idle_since(redis, &instance.instance_id, 0).await
}

// ── Utility ───────────────────────────────────────────────────────────────────

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub async fn health_ping(redis: &redis::Client) -> Result<()> {
    with_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map(|_| ())
            .map_err(anyhow::Error::from)
    })
    .await
}
