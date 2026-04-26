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
pub struct RoomRoute {
    pub signaling_instance_id: String,
}

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("room not found")]
    RoomNotFound,
    #[error("room route is invalid")]
    InvalidRoomRoute,
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
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

pub async fn reserve_nickname(client: &Client, nickname: &str) -> Result<(), RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let added: usize = conn.sadd(AUTH_NICKS_KEY, nickname).await?;
    if added == 0 {
        return Err(RedisError::NicknameTaken);
    }
    Ok(())
}

pub async fn release_nickname(client: &Client, nickname: &str) -> Result<(), RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn.srem(AUTH_NICKS_KEY, nickname).await?;
    Ok(())
}

pub async fn session_create(
    client: &Client,
    session_id: &str,
    nickname: &str,
) -> Result<(), RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let key = session_key(session_id);
    let _: usize = conn.hset(&key, "nickname", nickname).await?;
    let _: usize = conn.hset(&key, "room_id", "").await?;
    let _: usize = conn.hset(&key, "pending_room_id", "").await?;
    Ok(())
}

pub async fn session_delete(client: &Client, session_id: &str) -> Result<(), RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn.del(session_key(session_id)).await?;
    Ok(())
}

pub async fn get_room_route(client: &Client, room_id: &str) -> Result<RoomRoute, RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(room_meta_key(room_id)).await?;
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
}

pub async fn load_signaling_loads(client: &Client) -> Result<HashMap<String, f64>, RedisError> {
    let mut conn = client.get_connection_manager().await?;
    let entries: Vec<(String, f64)> = conn.zrange_withscores(INSTANCE_LOAD_KEY, 0, -1).await?;
    Ok(entries.into_iter().collect())
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

fn room_meta_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:meta")
}
