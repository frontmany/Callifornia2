use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use redis::AsyncCommands;
use tokio::time::timeout;

use crate::models::{RoomBinding, SfuCandidate, SfuInstance, WaitingRequestRecord};
use crate::util::unix_now;

static OP_TIMEOUT: OnceLock<Duration> = OnceLock::new();

const SFU_INSTANCES_KEY: &str = "room_manager:sfu_instances";
const SFU_ROOM_LOAD_KEY: &str = "room_manager:sfu_room_load";
const WAITING_REQUESTS_KEY: &str = "room_manager:waiting_requests";
const ROOM_PREFIX: &str = "room_manager:room:";

pub fn set_op_timeout(value: Duration) {
    let _ = OP_TIMEOUT.set(value);
}

fn op_timeout() -> Duration {
    OP_TIMEOUT.get().copied().unwrap_or(Duration::from_secs(2))
}

async fn with_op_timeout<T, F>(fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match timeout(op_timeout(), fut).await {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!("redis operation timed out")),
    }
}

pub async fn init_redis(redis_url: &str, _connect_timeout: Duration) -> Result<redis::Client> {
    let client = redis::Client::open(redis_url).context("create redis client")?;
    let mut conn = client.get_connection_manager().await?;
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .context("redis ping failed")?;
    Ok(client)
}

pub async fn select_least_loaded_instance(redis: &redis::Client) -> Result<Option<SfuInstance>> {
    let instances = list_sfu_instances(redis).await?;
    let room_loads = load_sfu_room_loads(redis).await?;
    let now = unix_now();

    let selected = instances
        .into_iter()
        .filter(|instance| {
            instance.alive
                && now - instance.last_ping_unix <= 30
                && instance.state == "ready"
                && room_loads
                    .get(&instance.instance_id)
                    .copied()
                    .unwrap_or_default()
                    < f64::from(instance.max_rooms)
        })
        .min_by(|left, right| {
            let left_load = room_loads
                .get(&left.instance_id)
                .copied()
                .unwrap_or_default();
            let right_load = room_loads
                .get(&right.instance_id)
                .copied()
                .unwrap_or_default();
            left_load
                .partial_cmp(&right_load)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    Ok(selected)
}

pub async fn has_provisioning_instance(redis: &redis::Client) -> Result<bool> {
    Ok(list_sfu_instances(redis)
        .await?
        .into_iter()
        .any(|instance| matches!(instance.state.as_str(), "starting" | "provisioning")))
}

pub async fn assign_room_to_instance(
    redis: &redis::Client,
    room_id: &str,
    signaling_owner_id: &str,
    signaling_owner_host: &str,
    signaling_owner_port: u16,
    instance: &SfuInstance,
) -> Result<RoomBinding> {
    with_op_timeout(async move {
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

        Ok(RoomBinding {
            room_id: room_id.to_owned(),
            owner_id: signaling_owner_id.to_owned(),
            owner_host: signaling_owner_host.to_owned(),
            owner_port: signaling_owner_port,
            state: "active".to_owned(),
            sfu_instance_id: Some(instance.instance_id.clone()),
            sfu_grpc_addr: Some(instance.grpc_addr.clone()),
        })
    })
    .await?;

    increment_sfu_room_load(redis, &instance.instance_id).await?;

    Ok(RoomBinding {
        room_id: room_id.to_owned(),
        owner_id: signaling_owner_id.to_owned(),
        owner_host: signaling_owner_host.to_owned(),
        owner_port: signaling_owner_port,
        state: "active".to_owned(),
        sfu_instance_id: Some(instance.instance_id.clone()),
        sfu_grpc_addr: Some(instance.grpc_addr.clone()),
    })
}

pub async fn get_room_assignment(
    redis: &redis::Client,
    room_id: &str,
) -> Result<Option<RoomBinding>> {
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(room_binding_key(room_id)).await?;
        if values.is_empty() {
            return Ok(None);
        }

        let owner_host = values.get("owner_host").cloned().unwrap_or_default();
        let owner_port = values
            .get("owner_port")
            .and_then(|value| value.parse().ok())
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
                .unwrap_or_else(|| "allocating".to_owned()),
            sfu_instance_id: values
                .get("sfu_instance_id")
                .cloned()
                .filter(|value| !value.is_empty()),
            sfu_grpc_addr: values
                .get("sfu_grpc_addr")
                .cloned()
                .filter(|value| !value.is_empty()),
        }))
    })
    .await
}

pub async fn register_provisioning_instance(
    redis: &redis::Client,
    candidate: &SfuCandidate,
) -> Result<()> {
    let instance = SfuInstance {
        instance_id: candidate.instance_id.clone(),
        grpc_addr: candidate.grpc_addr.clone(),
        max_rooms: candidate.max_rooms,
        alive: false,
        provisioned: true,
        state: "starting".to_owned(),
        last_ping_unix: 0,
        idle_since_unix: unix_now(),
    };
    let payload = serde_json::to_string(&instance)?;
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn
            .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
            .await?;
        let _: usize = conn
            .zadd(SFU_ROOM_LOAD_KEY, &instance.instance_id, 0)
            .await?;
        Ok(())
    })
    .await
}

pub async fn list_sfu_instances(redis: &redis::Client) -> Result<Vec<SfuInstance>> {
    with_op_timeout(async move {
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
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let entries: Vec<(String, f64)> = conn.zrange_withscores(SFU_ROOM_LOAD_KEY, 0, -1).await?;
        Ok(entries.into_iter().collect())
    })
    .await
}

pub async fn increment_sfu_room_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: f64 = conn.zincr(SFU_ROOM_LOAD_KEY, instance_id, 1).await?;
        Ok(())
    })
    .await?;
    update_idle_since(redis, instance_id, 0).await
}

pub async fn decrement_sfu_room_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let next = with_op_timeout(async move {
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


pub async fn get_sfu_instance(
    redis: &redis::Client,
    instance_id: &str,
) -> Result<Option<SfuInstance>> {
    with_op_timeout(async move {
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
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn
            .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
            .await?;
        Ok(())
    })
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error("waiting queue is full")]
    QueueFull,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub async fn enqueue_waiting_request(
    redis: &redis::Client,
    max_waiting_requests: usize,
    record: WaitingRequestRecord,
) -> Result<(), EnqueueError> {
    let current = waiting_request_count(redis).await?;
    if current >= max_waiting_requests {
        return Err(EnqueueError::QueueFull);
    }

    if let Some(existing) = get_room_assignment(redis, &record.room_id).await? {
        if existing.state == "allocating" {
            return Ok(());
        }
    }

    let payload = serde_json::to_string(&record).map_err(anyhow::Error::from)?;
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let _: usize = conn.rpush(WAITING_REQUESTS_KEY, payload).await?;

        let meta_key = room_binding_key(&record.room_id);
        let _: usize = conn
            .hset(&meta_key, "owner_host", &record.signaling_owner_host)
            .await?;
        let _: usize = conn
            .hset(&meta_key, "owner_port", record.signaling_owner_port)
            .await?;
        let _: usize = conn
            .hset(&meta_key, "signaling_owner_id", &record.signaling_owner_id)
            .await?;
        let _: usize = conn.hset(&meta_key, "room_state", "allocating").await?;
        Ok(())
    })
    .await
    .map_err(EnqueueError::from)
}

pub async fn waiting_request_count(redis: &redis::Client) -> Result<usize> {
    with_op_timeout(async move {
        let mut conn = redis.get_connection_manager().await?;
        let len: usize = conn.llen(WAITING_REQUESTS_KEY).await?;
        Ok(len)
    })
    .await
}

pub async fn count_active_rooms(redis: &redis::Client) -> Result<usize> {
    with_op_timeout(async move {
        let mut cursor = 0;
        let mut total = 0;
        let mut conn = redis.get_connection_manager().await?;

        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("room_manager:room:*:binding")
                .query_async(&mut conn)
                .await?;
            total += keys.len();
            if next == 0 {
                break;
            }
            cursor = next;
        }
        Ok(total)
    })
    .await
}

pub async fn delete_room_assignment(redis: &redis::Client, room_id: &str) -> Result<()> {
    with_op_timeout(async move {
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

fn room_binding_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:binding")
}

