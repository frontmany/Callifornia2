use anyhow::{Context, Result};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
const AUTH_NICKS_KEY: &str = "signaling:auth:nicks";
const INSTANCE_LOAD_KEY: &str = "signaling:instance_load";
const SESSION_PREFIX: &str = "signaling:session:";
const ROOM_PREFIX: &str = "signaling:room:";

#[derive(Debug, Clone)]
pub struct SessionState {
    pub nickname: String,
    pub room_id: Option<String>,
    pub pending_room_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RoomTarget {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct RoomRoute {
    pub target: RoomTarget,
    pub room_state: String,
    pub sfu_grpc_addr: Option<String>,
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
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
}

pub async fn release_nickname(client: &Client, nickname: &str) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn.srem(AUTH_NICKS_KEY, nickname).await?;
    Ok(())
}

pub async fn session_store(
    client: &Client,
    session_id: &str,
    session: &SessionState,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let key = session_key(session_id);
    let _: usize = conn.hset(&key, "nickname", &session.nickname).await?;
    let _: usize = conn
        .hset(&key, "room_id", session.room_id.as_deref().unwrap_or(""))
        .await?;
    let _: usize = conn
        .hset(
            &key,
            "pending_room_id",
            session.pending_room_id.as_deref().unwrap_or(""),
        )
        .await?;
    Ok(())
}

pub async fn session_get(
    client: &Client,
    session_id: &str,
) -> Result<Option<SessionState>, RedisRoomError> {
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
    let pending_room_id = values
        .get("pending_room_id")
        .filter(|value| !value.is_empty())
        .cloned();
    Ok(Some(SessionState {
        nickname,
        room_id,
        pending_room_id,
    }))
}

pub async fn session_set_room(
    client: &Client,
    session_id: &str,
    room_id: Option<&str>,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn
        .hset(session_key(session_id), "room_id", room_id.unwrap_or(""))
        .await?;
    Ok(())
}

pub async fn session_set_pending_room(
    client: &Client,
    session_id: &str,
    room_id: Option<&str>,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn
        .hset(
            session_key(session_id),
            "pending_room_id",
            room_id.unwrap_or(""),
        )
        .await?;
    Ok(())
}

pub async fn session_delete(client: &Client, session_id: &str) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn.del(session_key(session_id)).await?;
    Ok(())
}

pub async fn increment_instance_load(
    client: &Client,
    instance_id: &str,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: f64 = conn.zincr(INSTANCE_LOAD_KEY, instance_id, 1).await?;
    Ok(())
}

pub async fn decrement_instance_load(
    client: &Client,
    instance_id: &str,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let current: f64 = conn
        .zscore(INSTANCE_LOAD_KEY, instance_id)
        .await
        .unwrap_or(0.0);
    let next = if current <= 0.0 { 0.0 } else { current - 1.0 };
    let _: usize = conn.zadd(INSTANCE_LOAD_KEY, instance_id, next).await?;
    Ok(())
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

pub async fn health_ping(client: &Client) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .map(|_| ())
        .map_err(RedisRoomError::from)
}

async fn connect_redis(client: &Client, connect_timeout: Duration) -> Result<ConnectionManager> {
    timeout(connect_timeout, client.get_connection_manager())
        .await
        .context("redis connect timeout")?
        .context("redis connection manager failed")
}

#[allow(dead_code)]
pub async fn get_room_target(client: &Client, room_id: &str) -> Result<RoomTarget, RedisRoomError> {
    Ok(get_room_route(client, room_id).await?.target)
}

pub async fn get_room_route(client: &Client, room_id: &str) -> Result<RoomRoute, RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(room_meta_key(room_id)).await?;
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
        target: RoomTarget { host, port },
        room_state: values
            .get("room_state")
            .cloned()
            .unwrap_or_else(|| "active".to_owned()),
        sfu_grpc_addr: values
            .get("sfu_grpc_addr")
            .cloned()
            .filter(|value| !value.is_empty()),
    })
}

pub async fn upsert_room_route(
    client: &Client,
    room_id: &str,
    target: &RoomTarget,
    room_state: &str,
    sfu_grpc_addr: Option<&str>,
) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let key = room_meta_key(room_id);
    let _: () = redis::pipe()
        .atomic()
        .hset(&key, "owner_host", &target.host)
        .hset(&key, "owner_port", target.port)
        .hset(&key, "room_state", room_state)
        .hset(&key, "sfu_grpc_addr", sfu_grpc_addr.unwrap_or(""))
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub async fn join_room(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<Vec<String>, RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;

    let exists: bool = conn.exists(room_meta_key(room_id)).await?;
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
}

pub async fn leave_room(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<LeaveRoomResult, RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let members_key = room_members_key(room_id);
    let meta_key = room_meta_key(room_id);

    let exists: bool = conn.exists(&meta_key).await?;
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
}

#[allow(dead_code)]
pub async fn delete_room(client: &Client, room_id: &str) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let _: () = redis::pipe()
        .atomic()
        .del(room_members_key(room_id))
        .ignore()
        .del(room_meta_key(room_id))
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn clear_signaling_keys(client: &Client) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("signaling:*")
        .query_async(&mut conn)
        .await?;
    if keys.is_empty() {
        return Ok(());
    }

    let _: usize = conn.del(keys).await?;
    Ok(())
}

fn session_key(session_id: &str) -> String {
    format!("{SESSION_PREFIX}{session_id}")
}

fn room_meta_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:meta")
}

fn room_members_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:members")
}
