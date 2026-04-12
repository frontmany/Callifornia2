use std::time::Duration;
use anyhow::{Context, Result};
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use redis::Client;
use thiserror::Error;
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum RedisRoomError {
    #[error("room not found")]
    RoomNotFound,
    #[error("nickname already taken")]
    NicknameTaken,
    #[error("participant not in room")]
    ParticipantNotInRoom,
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

async fn connect_redis(client: &Client, connect_timeout: Duration) -> Result<ConnectionManager> {
    timeout(connect_timeout, client.get_connection_manager())
        .await
        .context("redis connect timeout")?
        .context("redis connection manager failed")
}

pub async fn create_room(client: &Client, nickname: &str) -> Result<String, RedisRoomError> {
    let room_id = Uuid::new_v4().to_string();

    let mut conn = client.get_connection_manager().await?;
    let _: usize = conn.sadd(&room_id, nickname).await?;

    Ok(room_id)
}

pub async fn join_room(
    client: &Client,
    room_id: &str,
    nickname: &str,
) -> Result<Vec<String>, RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;

    let exists: bool = conn.exists(room_id).await?;
    if !exists {
        return Err(RedisRoomError::RoomNotFound);
    }

    let already_taken: bool = conn.sismember(room_id, nickname).await?;
    if already_taken {
        return Err(RedisRoomError::NicknameTaken);
    }

    let _: usize = conn.sadd(room_id, nickname).await?;
    
    let mut participants: Vec<String> = conn.smembers(room_id).await?;
    participants.retain(|name| name != nickname);

    Ok(participants)
}

pub async fn leave_room(client: &Client, room_id: &str, nickname: &str) -> Result<(), RedisRoomError> {
    let mut conn = client.get_connection_manager().await?;

    let exists: bool = conn.exists(room_id).await?;
    if !exists {
        return Err(RedisRoomError::RoomNotFound);
    }

    let removed: usize = conn.srem(room_id, nickname).await?;
    if removed == 0 {
        return Err(RedisRoomError::ParticipantNotInRoom);
    }

    let remaining: usize = conn.scard(room_id).await?;
    if remaining == 0 {
        let _: usize = conn.del(room_id).await?;
    }

    Ok(())
}
