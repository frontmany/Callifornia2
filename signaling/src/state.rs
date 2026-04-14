use std::sync::Arc;

use tracing::warn;

use crate::config::Config;
use crate::message::ServerMessage;
use crate::peer_registry::{DeliveryStatus, PeerRegistry};
use crate::redis::{self, RedisRoomError};
use crate::room_manager::RoomManagerError;
use crate::session_registry::SessionRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Ok,
    Down,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeHealth {
    pub redis: ServiceState,
    pub room_manager: ServiceState,
}

impl Default for RuntimeHealth {
    fn default() -> Self {
        Self {
            redis: ServiceState::Ok,
            room_manager: ServiceState::Ok,
        }
    }
}

#[derive(Clone)]
pub struct State {
    pub config: Arc<Config>,
    pub redis: ::redis::Client,
    pub sfu: crate::sfu::Registry,
    pub room_manager: crate::room_manager::Client,
    pub peers: PeerRegistry,
    pub sessions: SessionRegistry,
    health: Arc<tokio::sync::RwLock<RuntimeHealth>>,
}

impl State {
    pub fn new(
        config: Arc<Config>,
        redis: ::redis::Client,
        sfu: crate::sfu::Registry,
        room_manager: crate::room_manager::Client,
        peers: PeerRegistry,
        sessions: SessionRegistry,
    ) -> Self {
        Self {
            config,
            redis,
            sfu,
            room_manager,
            peers,
            sessions,
            health: Arc::new(tokio::sync::RwLock::new(RuntimeHealth::default())),
        }
    }

    pub async fn health(&self) -> RuntimeHealth {
        *self.health.read().await
    }

    pub async fn set_redis_state(&self, state: ServiceState) {
        self.health.write().await.redis = state;
    }

    pub async fn is_redis_available(&self) -> bool {
        self.health.read().await.redis == ServiceState::Ok
    }

    pub async fn detach_peer(&self, room_id: &str, nickname: &str, reason: &str) {
        let route = if self.is_redis_available().await {
            match redis::get_room_route(&self.redis, room_id).await {
                Ok(route) => Some(route),
                Err(RedisRoomError::RoomNotFound) => None,
                Err(err) => {
                    warn!(error = %err, room_id = %room_id, "failed to resolve room route before detach");
                    None
                }
            }
        } else {
            None
        };

        self.peers.unregister(room_id, nickname).await;

        let mut room_empty = false;
        if self.is_redis_available().await {
            match redis::leave_room(&self.redis, room_id, nickname).await {
                Ok(result) => {
                    room_empty = result.room_empty;
                }
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
        }

        if let Some(sfu_addr) = route.and_then(|route| route.sfu_grpc_addr) {
            if let Err(err) = self
                .sfu
                .delete_peer(&sfu_addr, room_id, nickname, reason)
                .await
            {
                warn!(
                    error = %err,
                    sfu_addr = %sfu_addr,
                    room_id = %room_id,
                    nickname = %nickname,
                    "failed to detach peer from SFU"
                );
            }
        }

        if room_empty {
            if let Err(err) = self.room_manager.close_room(room_id).await {
                warn!(error = %err, room_id = %room_id, "failed to close empty room in room manager");
            }
        }
    }

    pub async fn close_rooms_due_to_sfu_addr(&self, sfu_addr: &str, reason: &str) {
        let rooms = self.peers.snapshot_rooms().await;
        for (room_id, participants) in rooms {
            let Ok(route) = redis::get_room_route(&self.redis, &room_id).await else {
                continue;
            };
            if route.sfu_grpc_addr.as_deref() != Some(sfu_addr) {
                continue;
            }

            let stale = self
                .peers
                .broadcast_to_room(
                    &room_id,
                    None,
                    ServerMessage::RoomClosed {
                        room_id: room_id.clone(),
                        reason: reason.to_owned(),
                    },
                )
                .await;

            for nickname in participants {
                self.peers.unregister(&room_id, &nickname).await;
            }

            for nickname in stale {
                self.peers.unregister(&room_id, &nickname).await;
            }

            let affected_sessions = self.sessions.clear_room(&room_id).await;

            if self.is_redis_available().await {
                if let Err(err) = self.room_manager.close_room(&room_id).await {
                    warn!(error = %err, room_id = %room_id, "failed to close room after sfu down");
                }
                for session_id in affected_sessions {
                    if let Err(err) = redis::session_set_room(&self.redis, &session_id, None).await
                    {
                        warn!(
                            error = %err,
                            room_id = %room_id,
                            session_id = %session_id,
                            "failed to clear session room after sfu down"
                        );
                    }
                }
            }
        }
    }

    pub async fn notify_participants_left(
        &self,
        room_id: &str,
        nickname: &str,
        participants: &[String],
    ) {
        for participant in participants {
            if participant == nickname {
                continue;
            }
            match self
                .peers
                .send_to_peer(
                    room_id,
                    participant,
                    ServerMessage::ParticipantLeft {
                        nickname: nickname.to_owned(),
                    },
                )
                .await
            {
                DeliveryStatus::Delivered | DeliveryStatus::Missing => {}
                DeliveryStatus::Stale => {
                    self.peers.unregister(room_id, participant).await;
                }
            }
        }
    }
}

impl From<RoomManagerError> for ServiceState {
    fn from(_: RoomManagerError) -> Self {
        ServiceState::Down
    }
}
