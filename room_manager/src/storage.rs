use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use redis::AsyncCommands;

use crate::models::{RoomAssignment, SfuCandidate, SfuInstanceRecord, WaitingRequestRecord};
use crate::util::unix_now;

const SFU_INSTANCES_KEY: &str = "room_manager:sfu_instances";
const SFU_LOAD_KEY: &str = "room_manager:sfu_load";
const WAITING_REQUESTS_KEY: &str = "room_manager:waiting_requests";
const ROOM_PREFIX: &str = "signaling:room:";

pub async fn init_redis(redis_url: &str, _connect_timeout: Duration) -> Result<redis::Client> {
    let client = redis::Client::open(redis_url).context("create redis client")?;
    let mut conn = client.get_connection_manager().await?;
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .context("redis ping failed")?;
    Ok(client)
}

pub async fn select_least_loaded_instance(redis: &redis::Client) -> Result<Option<SfuInstanceRecord>> {
    let instances = list_sfu_instances(redis).await?;
    let loads = load_sfu_loads(redis).await?;
    let now = unix_now();

    let selected = instances
        .into_iter()
        .filter(|instance| {
            instance.alive
                && now - instance.last_ping_unix <= 30
                && instance.state == "ready"
                && loads
                    .get(&instance.instance_id)
                    .copied()
                    .unwrap_or_default()
                    < f64::from(instance.max_clients)
        })
        .min_by(|left, right| {
            let left_load = loads.get(&left.instance_id).copied().unwrap_or_default();
            let right_load = loads.get(&right.instance_id).copied().unwrap_or_default();
            left_load
                .partial_cmp(&right_load)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    Ok(selected)
}

pub async fn assign_room_to_instance(
    redis: &redis::Client,
    room_id: &str,
    signaling_owner_id: &str,
    signaling_owner_host: &str,
    signaling_owner_port: u16,
    instance: &SfuInstanceRecord,
) -> Result<RoomAssignment> {
    let mut conn = redis.get_connection_manager().await?;
    let meta_key = room_meta_key(room_id);

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

    increment_sfu_load(redis, &instance.instance_id).await?;

    Ok(RoomAssignment {
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
) -> Result<Option<RoomAssignment>> {
    let mut conn = redis.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(room_meta_key(room_id)).await?;
    if values.is_empty() {
        return Ok(None);
    }

    let owner_host = values.get("owner_host").cloned().unwrap_or_default();
    let owner_port = values
        .get("owner_port")
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();

    Ok(Some(RoomAssignment {
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
}

pub async fn register_provisioning_instance(
    redis: &redis::Client,
    candidate: &SfuCandidate,
) -> Result<()> {
    // TODO(provisioner): это запись "instance starting" без фактического запуска процесса.
    // После интеграции с оркестратором состояние должно идти из реального lifecycle инстанса.
    let instance = SfuInstanceRecord {
        instance_id: candidate.instance_id.clone(),
        grpc_addr: candidate.grpc_addr.clone(),
        max_clients: candidate.max_clients,
        alive: false,
        provisioned: true,
        state: "starting".to_owned(),
        last_ping_unix: 0,
        idle_since_unix: unix_now(),
    };
    let payload = serde_json::to_string(&instance)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn
        .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
        .await?;
    let _: usize = conn.zadd(SFU_LOAD_KEY, &instance.instance_id, 0).await?;
    Ok(())
}

pub async fn delete_sfu_instance(redis: &redis::Client, instance_id: &str) -> Result<()> {
    // TODO(provisioner): перед удалением записи из Redis нужно останавливать реальный SFU инстанс
    // через внешний API и обрабатывать ошибки terminate (retries/compensation).
    let mut conn = redis.get_connection_manager().await?;
    let _: () = redis::pipe()
        .atomic()
        .hdel(SFU_INSTANCES_KEY, instance_id)
        .ignore()
        .zrem(SFU_LOAD_KEY, instance_id)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub async fn list_sfu_instances(redis: &redis::Client) -> Result<Vec<SfuInstanceRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(SFU_INSTANCES_KEY).await?;
    values
        .into_values()
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .collect()
}

pub async fn load_sfu_loads(redis: &redis::Client) -> Result<HashMap<String, f64>> {
    let mut conn = redis.get_connection_manager().await?;
    let entries: Vec<(String, f64)> = conn.zrange_withscores(SFU_LOAD_KEY, 0, -1).await?;
    Ok(entries.into_iter().collect())
}

pub async fn increment_sfu_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let _: f64 = conn.zincr(SFU_LOAD_KEY, instance_id, 1).await?;
    update_idle_since(redis, instance_id, 0).await
}

pub async fn decrement_sfu_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let current: f64 = conn.zscore(SFU_LOAD_KEY, instance_id).await.unwrap_or(0.0);
    let next = if current <= 0.0 { 0.0 } else { current - 1.0 };
    let _: usize = conn.zadd(SFU_LOAD_KEY, instance_id, next).await?;
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

pub async fn get_sfu_instance(
    redis: &redis::Client,
    instance_id: &str,
) -> Result<Option<SfuInstanceRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let payload: Option<String> = conn.hget(SFU_INSTANCES_KEY, instance_id).await?;
    payload
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .transpose()
}

pub async fn save_sfu_instance(redis: &redis::Client, instance: &SfuInstanceRecord) -> Result<()> {
    let payload = serde_json::to_string(instance)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn
        .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
        .await?;
    Ok(())
}

pub async fn enqueue_waiting_request(
    redis: &redis::Client,
    max_waiting_requests: usize,
    record: WaitingRequestRecord,
) -> Result<()> {
    let current = waiting_request_count(redis).await?;
    if current >= max_waiting_requests {
        // TODO(queue): сейчас переполнение очереди silently drops запрос.
        // Нужен явный сигнал вызывающему (ошибка/статус queue_full) + метрика.
        return Ok(());
    }

    // TODO(queue-idempotency): исключить дубликаты room_id в очереди ожидания,
    // например через дополнительный индекс/lock ключ в Redis.
    let payload = serde_json::to_string(&record)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn.rpush(WAITING_REQUESTS_KEY, payload).await?;

    let meta_key = room_meta_key(&record.room_id);
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
}

pub async fn pop_waiting_request(redis: &redis::Client) -> Result<Option<WaitingRequestRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let payload: Option<String> = conn.lpop(WAITING_REQUESTS_KEY, None).await?;
    payload
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .transpose()
}

pub async fn waiting_request_count(redis: &redis::Client) -> Result<usize> {
    let mut conn = redis.get_connection_manager().await?;
    let len: usize = conn.llen(WAITING_REQUESTS_KEY).await?;
    Ok(len)
}

pub async fn count_active_rooms(redis: &redis::Client) -> Result<usize> {
    let mut cursor = 0;
    let mut total = 0;
    let mut conn = redis.get_connection_manager().await?;

    loop {
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("signaling:room:*:meta")
            .query_async(&mut conn)
            .await?;
        total += keys.len();
        if next == 0 {
            break;
        }
        cursor = next;
    }
    Ok(total)
}

pub async fn delete_room_assignment(redis: &redis::Client, room_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let _: () = redis::pipe()
        .atomic()
        .del(room_meta_key(room_id))
        .ignore()
        .del(room_members_key(room_id))
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

fn room_meta_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:meta")
}

fn room_members_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:members")
}
