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
const SIGNALING_NODES_KEY: &str = "signaling:nodes";
const SESSION_PREFIX: &str = "signaling:session:";
const NICK_PREFIX: &str = "signaling:nick:";
const ROOM_BINDING_PREFIX: &str = "room_manager:room:";

const COMPARE_AND_DEL_LUA: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
else
    return 0
end
"#;

#[derive(Debug, Clone)]
pub struct RoomRoute {
    pub signaling_instance_id: String,
}

#[derive(Debug, Clone)]
pub struct SessionData {
    pub nickname: String,
}

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("room not found")]
    RoomNotFound,
    #[error("room route is invalid")]
    InvalidRoomRoute,
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

async fn with_op_timeout<T, F>(fut: F) -> Result<T, RedisError>
where
    F: std::future::Future<Output = Result<T, RedisError>>,
{
    match timeout(op_timeout(), fut).await {
        Ok(r) => r,
        Err(_) => Err(RedisError::Timeout),
    }
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

/// Reserves `nickname` for `session_id` until [`release_nick_lease`] or logout.
/// No Redis TTL on the nick key — the client keeps the reservation until explicit release.
pub async fn reserve_nickname(
    client: &Client,
    nickname: &str,
    session_id: &str,
) -> Result<(), RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let acquired: Option<String> = redis::cmd("SET")
            .arg(nick_key(nickname))
            .arg(session_id)
            .arg("NX")
            .query_async(&mut conn)
            .await?;

        match acquired.as_deref() {
            Some("OK") => Ok(()),
            _ => Err(RedisError::NicknameTaken),
        }
    })
    .await
}

/// Current holder of `nickname`, if any (`session_id` string).
pub async fn nick_owner_get(client: &Client, nickname: &str) -> Result<Option<String>, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let v: Option<String> = conn.get(nick_key(nickname)).await?;
        Ok(v)
    })
    .await
}

pub async fn release_nick_lease(
    client: &Client,
    nickname: &str,
    session_id: &str,
) -> Result<(), RedisError> {
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

pub async fn session_create(
    client: &Client,
    session_id: &str,
    nickname: &str,
    ttl: Duration,
) -> Result<(), RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let key = session_key(session_id);
        let _: usize = conn.hset(&key, "nickname", nickname).await?;
        let _: usize = conn.hset(&key, "room_id", "").await?;
        let _: usize = conn.hset(&key, "pending_room_id", "").await?;
        let _: bool = conn.expire(&key, ttl.as_secs() as i64).await?;
        Ok(())
    })
    .await
}

pub async fn session_delete(client: &Client, session_id: &str) -> Result<(), RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let _: usize = conn.del(session_key(session_id)).await?;
        Ok(())
    })
    .await
}

pub async fn session_get(client: &Client, session_id: &str) -> Result<Option<SessionData>, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let key = session_key(session_id);
        let nickname: Option<String> = conn.hget(&key, "nickname").await?;
        Ok(nickname
            .filter(|value| !value.is_empty())
            .map(|nickname| SessionData { nickname }))
    })
    .await
}

/// Refreshes TTL on the session hash key (slides expiry by full `ttl`).
pub async fn session_refresh_ttl(
    client: &Client,
    session_id: &str,
    ttl: Duration,
) -> Result<bool, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let ok: bool = conn
            .expire(session_key(session_id), ttl.as_secs() as i64)
            .await?;
        Ok(ok)
    })
    .await
}

pub async fn get_room_route(client: &Client, room_id: &str) -> Result<RoomRoute, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let values: HashMap<String, String> = conn.hgetall(room_binding_key(room_id)).await?;
        if values.is_empty() {
            return Err(RedisError::RoomNotFound);
        }
        let host = values
            .get("owner_host")
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(RedisError::InvalidRoomRoute)?;
        let port = values
            .get("owner_port")
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or(RedisError::InvalidRoomRoute)?;

        Ok(RoomRoute {
            signaling_instance_id: format!("{host}:{port}"),
        })
    })
    .await
}

pub async fn load_signaling_loads(client: &Client) -> Result<HashMap<String, f64>, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let entries: Vec<(String, f64)> = conn.zrange_withscores(INSTANCE_LOAD_KEY, 0, -1).await?;
        Ok(entries.into_iter().collect())
    })
    .await
}

pub async fn alive_signaling_nodes(
    client: &Client,
    stale_threshold: Duration,
) -> Result<Vec<String>, RedisError> {
    with_op_timeout(async move {
        let mut conn = client.get_connection_manager().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cutoff = now - stale_threshold.as_secs() as i64;
        let nodes: Vec<String> = conn
            .zrangebyscore(SIGNALING_NODES_KEY, cutoff, "+inf")
            .await?;
        Ok(nodes)
    })
    .await
}

pub async fn get_supervisor_status(
    client: &Client,
) -> Result<Option<control_store::models::SupervisorStatus>, RedisError> {
    control_store::storage::read_supervisor_status(client)
        .await
        .map_err(|e| RedisError::Redis(redis::RedisError::from(std::io::Error::other(e.to_string()))))
}

async fn connect_redis(client: &Client, connect_timeout: Duration) -> Result<ConnectionManager> {
    timeout(connect_timeout, client.get_connection_manager())
        .await
        .context("redis connect timeout")?
        .context("redis connection manager failed")
}

fn session_key(session_id: &str) -> String {
    format!("{SESSION_PREFIX}{session_id}")
}

fn nick_key(nickname: &str) -> String {
    format!("{NICK_PREFIX}{nickname}")
}

fn room_binding_key(room_id: &str) -> String {
    format!("{ROOM_BINDING_PREFIX}{room_id}:binding")
}
