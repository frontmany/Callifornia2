use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;

static OP_TIMEOUT: OnceLock<Duration> = OnceLock::new();
const INSTANCE_LOAD_KEY: &str = "signaling:instance_load";
const SESSION_PREFIX: &str = "signaling:session:";
const NICK_PREFIX: &str = "signaling:nick:";
const ROOM_BINDING_PREFIX: &str = "room:";
const ROOM_MEMBERS_PREFIX: &str = "signaling:room:";
const SFU_INSTANCES_KEY: &str = "sfu:instances";
const SFU_ROOM_LOAD_KEY: &str = "sfu:room_load";

const COMPARE_AND_DEL_LUA: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
else
    return 0
end
"#;

const ASSIGN_ROOM_TO_SFU_LUA: &str = r#"
local instances = redis.call('HGETALL', KEYS[1])
local now = tonumber(ARGV[6])
local stale_after = tonumber(ARGV[7])
local best_id = nil
local best_grpc = nil
local best_load = nil
local best_payload = nil

for i = 1, #instances, 2 do
    local payload = instances[i + 1]
    local inst = cjson.decode(payload)
    local load = tonumber(redis.call('ZSCORE', KEYS[2], inst.instance_id) or '0')
    local last_ping = tonumber(inst.last_ping_unix or 0)
    local max_rooms = tonumber(inst.max_rooms or 0)

    if inst.alive == true
        and inst.state == 'ready'
        and now - last_ping <= stale_after
        and load < max_rooms
        and (best_load == nil or load < best_load)
    then
        best_id = inst.instance_id
        best_grpc = inst.grpc_addr
        best_load = load
        best_payload = inst
    end
end

if best_id == nil then
    return {}
end

if redis.call('EXISTS', KEYS[3]) == 1 then
    return {'__ROOM_EXISTS__', ''}
end

redis.call('HSET', KEYS[3],
    'owner_host', ARGV[2],
    'owner_port', ARGV[3],
    'signaling_owner_id', ARGV[4],
    'room_state', 'active',
    'sfu_instance_id', best_id,
    'sfu_grpc_addr', best_grpc
)
redis.call('ZINCRBY', KEYS[2], 1, best_id)
best_payload.idle_since_unix = 0
redis.call('HSET', KEYS[1], best_id, cjson.encode(best_payload))

return {best_id, best_grpc}
"#;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub nickname: String,
    pub room_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RoomRoute {
    pub owner_host: String,
    pub owner_port: u16,
    pub room_state: String,
    pub sfu_grpc_addr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SfuAssignment {
    pub sfu_grpc_addr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaveRoomResult {
    pub room_empty: bool,
}

#[derive(Debug, Error)]
pub enum RedisRoomError {
    #[error("room not found")]
    RoomNotFound,
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("participant not in room")]
    ParticipantNotInRoom,
    #[error("room target is invalid")]
    InvalidRoomTarget,
    #[error("no ready SFU capacity")]
    NoSfuCapacity,
    #[error("room already exists")]
    RoomAlreadyExists,
    #[error("redis operation timed out")]
    Timeout,
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
}

pub fn set_op_timeout(value: Duration) {
    let _ = OP_TIMEOUT.set(value);
}

fn op_timeout() -> Duration {
    OP_TIMEOUT.get().copied().unwrap_or(Duration::from_secs(2))
}

async fn with_op_timeout<T, F>(fut: F) -> Result<T, RedisRoomError>
where
    F: std::future::Future<Output = Result<T, RedisRoomError>>,
{
    match timeout(op_timeout(), fut).await {
        Ok(r) => r,
        Err(_) => Err(RedisRoomError::Timeout),
    }
}

pub async fn consume_jti(
    client: &Client,
    jti: &str,
    ttl: Duration,
) -> Result<bool, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let acquired: Option<String> = redis::cmd("SET")
            .arg(format!("signaling:jti:{jti}"))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl.as_secs().max(1))
            .query_async(&mut conn)
            .await?;
        Ok(acquired.as_deref() == Some("OK"))
    })
    .await
}

pub async fn session_set_owner(
    client: &Client,
    session_id: &str,
    owner_id: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn
            .hset(session_key(session_id), "signaling_owner_id", owner_id)
            .await?;
        Ok(())
    })
    .await
}

pub async fn acquire_session_lock(
    client: &Client,
    session_id: &str,
    owner: &str,
    ttl: Duration,
) -> Result<bool, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let acquired: Option<String> = redis::cmd("SET")
            .arg(session_lock_key(session_id))
            .arg(owner)
            .arg("NX")
            .arg("PX")
            .arg(ttl.as_millis() as u64)
            .query_async(&mut conn)
            .await?;
        Ok(acquired.as_deref() == Some("OK"))
    })
    .await
}

pub async fn release_session_lock(
    client: &Client,
    session_id: &str,
    owner: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: i64 = redis::Script::new(COMPARE_AND_DEL_LUA)
            .key(session_lock_key(session_id))
            .arg(owner)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

pub async fn release_nick_lease(
    client: &Client,
    nickname: &str,
    session_id: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: i64 = redis::Script::new(COMPARE_AND_DEL_LUA)
            .key(nick_key(nickname))
            .arg(session_id)
            .invoke_async(&mut conn)
            .await?;
        Ok(())
    })
    .await
}

pub async fn renew_session_ttl(
    client: &Client,
    session_id: &str,
    ttl: Duration,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: bool = conn
            .expire(session_key(session_id), ttl.as_secs() as i64)
            .await?;
        Ok(())
    })
    .await
}

pub async fn session_get(
    client: &Client,
    session_id: &str,
) -> Result<Option<SessionState>, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(session_key(session_id)).await?;
        if values.is_empty() {
            return Ok(None);
        }

        let nickname = values.get("nickname").cloned().unwrap_or_default();
        let room_id = values
            .get("room_id")
            .filter(|value| !value.is_empty())
            .cloned();
        Ok(Some(SessionState { nickname, room_id }))
    })
    .await
}

pub async fn session_set_room(
    client: &Client,
    session_id: &str,
    room_id: Option<&str>,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn
            .hset(session_key(session_id), "room_id", room_id.unwrap_or(""))
            .await?;
        Ok(())
    })
    .await
}

pub async fn session_delete(client: &Client, session_id: &str) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn.del(session_key(session_id)).await?;
        Ok(())
    })
    .await
}

pub async fn increment_instance_load(
    client: &Client,
    instance_id: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: f64 = conn.zincr(INSTANCE_LOAD_KEY, instance_id, 1).await?;
        Ok(())
    })
    .await
}

pub async fn decrement_instance_load(
    client: &Client,
    instance_id: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let current: Option<f64> = conn.zscore(INSTANCE_LOAD_KEY, instance_id).await?;
        let current = current.unwrap_or(0.0);
        let next = if current <= 1.0 { 0.0 } else { current - 1.0 };
        let _: usize = conn.zadd(INSTANCE_LOAD_KEY, instance_id, next).await?;
        Ok(())
    })
    .await
}

pub async fn init_redis(redis_url: &str, connect_timeout: Duration) -> Result<Client> {
    let client = Client::open(redis_url).context("create redis client")?;
    let mut conn = connect_redis(&client, connect_timeout).await?;

    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .context("redis ping failed")?;

    Ok(client)
}

async fn connect_redis(client: &Client, connect_timeout: Duration) -> Result<ConnectionManager> {
    timeout(connect_timeout, client.get_connection_manager())
        .await
        .context("redis connect timeout")?
        .context("redis connection manager failed")
}

pub async fn get_room_route(client: &Client, room_id: &str) -> Result<RoomRoute, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(room_binding_key(room_id)).await?;
        if values.is_empty() {
            return Err(RedisRoomError::RoomNotFound);
        }

        let host = values
            .get("owner_host")
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(RedisRoomError::InvalidRoomTarget)?;
        let port = values
            .get("owner_port")
            .ok_or(RedisRoomError::InvalidRoomTarget)?
            .parse()
            .map_err(|_| RedisRoomError::InvalidRoomTarget)?;

        Ok(RoomRoute {
            owner_host: host,
            owner_port: port,
            room_state: values
                .get("room_state")
                .cloned()
                .unwrap_or_else(|| "active".to_owned()),
            sfu_grpc_addr: values
                .get("sfu_grpc_addr")
                .cloned()
                .filter(|value| !value.is_empty()),
        })
    })
    .await
}

pub async fn assign_room_to_least_loaded_sfu(
    client: &Client,
    room_id: &str,
    signaling_owner_id: &str,
    signaling_owner_host: &str,
    signaling_owner_port: u16,
) -> Result<SfuAssignment, RedisRoomError> {
    let now = unix_now();
    let assignment = with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let result: Vec<String> = redis::Script::new(ASSIGN_ROOM_TO_SFU_LUA)
            .key(SFU_INSTANCES_KEY)
            .key(SFU_ROOM_LOAD_KEY)
            .key(room_binding_key(room_id))
            .arg(room_id)
            .arg(signaling_owner_host)
            .arg(signaling_owner_port)
            .arg(signaling_owner_id)
            .arg("active")
            .arg(now)
            .arg(30_i64)
            .invoke_async(&mut conn)
            .await?;

        if result.is_empty() {
            return Ok(None);
        }
        if result.first().map(String::as_str) == Some("__ROOM_EXISTS__") {
            return Err(RedisRoomError::RoomAlreadyExists);
        }
        let sfu_grpc_addr = result
            .get(1)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(RedisRoomError::InvalidRoomTarget)?;
        Ok(Some(SfuAssignment { sfu_grpc_addr }))
    })
    .await?;

    assignment.ok_or(RedisRoomError::NoSfuCapacity)
}

pub async fn join_room(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<Vec<String>, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;

        let exists: bool = conn.exists(room_binding_key(room_id)).await?;
        if !exists {
            return Err(RedisRoomError::RoomNotFound);
        }

        let members_key = room_members_key(room_id);
        let already_taken: bool = conn.sismember(&members_key, nickname).await?;
        if already_taken {
            return Err(RedisRoomError::NicknameTaken);
        }

        let _: usize = conn.sadd(&members_key, nickname).await?;

        let mut participants: Vec<String> = conn.smembers(&members_key).await?;
        participants.retain(|name| name != nickname);

        Ok(participants)
    })
    .await
}

pub async fn srem_member(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn.srem(room_members_key(room_id), nickname).await?;
        Ok(())
    })
    .await
}

pub async fn leave_room(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<LeaveRoomResult, RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let members_key = room_members_key(room_id);
        let binding_key = room_binding_key(room_id);

        let exists: bool = conn.exists(&binding_key).await?;
        if !exists {
            return Err(RedisRoomError::RoomNotFound);
        }

        let removed: usize = conn.srem(&members_key, nickname).await?;
        if removed == 0 {
            return Err(RedisRoomError::ParticipantNotInRoom);
        }

        let remaining: usize = conn.scard(&members_key).await?;
        if remaining == 0 {
            let _: usize = conn.del(&members_key).await?;
        }

        Ok(LeaveRoomResult {
            room_empty: remaining == 0,
        })
    })
    .await
}

pub async fn close_room_binding(client: &Client, room_id: &str) -> Result<bool, RedisRoomError> {
    let instance_id = with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let binding_key = room_binding_key(room_id);
        let instance_id: Option<String> = conn.hget(&binding_key, "sfu_instance_id").await?;
        let deleted: usize = conn.del(&binding_key).await?;
        Ok(if deleted == 0 { None } else { instance_id })
    })
    .await?;

    let Some(instance_id) = instance_id else {
        return Ok(false);
    };

    decrement_sfu_room_load(client, &instance_id).await?;
    Ok(true)
}

async fn decrement_sfu_room_load(client: &Client, instance_id: &str) -> Result<(), RedisRoomError> {
    let next = with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let current: Option<f64> = conn.zscore(SFU_ROOM_LOAD_KEY, instance_id).await?;
        let current = current.unwrap_or(0.0);
        let next = if current <= 1.0 { 0.0 } else { current - 1.0 };
        let _: usize = conn.zadd(SFU_ROOM_LOAD_KEY, instance_id, next).await?;
        Ok(next)
    })
    .await?;

    if next == 0.0 {
        update_sfu_idle_since(client, instance_id, unix_now()).await?;
    }
    Ok(())
}

async fn update_sfu_idle_since(
    client: &Client,
    instance_id: &str,
    idle_since_unix: i64,
) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let payload: Option<String> = conn.hget(SFU_INSTANCES_KEY, instance_id).await?;
        let Some(payload) = payload else {
            return Ok(());
        };
        let mut json: serde_json::Value =
            serde_json::from_str(&payload).map_err(|_| RedisRoomError::InvalidRoomTarget)?;
        json["idle_since_unix"] = serde_json::Value::from(idle_since_unix);
        let _: usize = conn
            .hset(SFU_INSTANCES_KEY, instance_id, json.to_string())
            .await?;
        Ok(())
    })
    .await
}

pub async fn delete_room(client: &Client, room_id: &str) -> Result<(), RedisRoomError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn.del(room_members_key(room_id)).await?;
        Ok(())
    })
    .await
}

fn session_key(session_id: &str) -> String {
    format!("{SESSION_PREFIX}{session_id}")
}

fn session_lock_key(session_id: &str) -> String {
    format!("{SESSION_PREFIX}{session_id}:lock")
}

fn nick_key(nickname: &str) -> String {
    format!("{NICK_PREFIX}{nickname}")
}

fn room_binding_key(room_id: &str) -> String {
    format!("{ROOM_BINDING_PREFIX}{room_id}:binding")
}

fn room_members_key(room_id: &str) -> String {
    format!("{ROOM_MEMBERS_PREFIX}{room_id}:members")
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
