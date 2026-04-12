use std::sync::Arc;

use crate::config::Config;
use crate::peer_registry::PeerRegistry;
use crate::redis::{self, RedisRoomError};
use tracing::warn;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub redis: ::redis::Client,
    pub sfu: crate::sfu::Client,
    pub peers: PeerRegistry,
}

impl AppState {
    pub async fn detach_peer(&self, room_id: &str, nickname: &str, reason: &str) {
        self.peers.unregister(room_id, nickname).await;

        match redis::leave_room(&self.redis, room_id, nickname).await {
            Ok(()) => {}
            Err(RedisRoomError::RoomNotFound | RedisRoomError::ParticipantNotInRoom) => {}
            Err(err) => {
                warn!(
                    error = %err,
                    room_id = %room_id,
                    nickname = %nickname,
                    "failed to detach peer from redis"
                );
            }
        }

        if let Err(err) = self.sfu.delete_peer(room_id, nickname, reason).await {
            warn!(
                error = %err,
                room_id = %room_id,
                nickname = %nickname,
                "failed to detach peer from SFU"
            );
        }
    }
}