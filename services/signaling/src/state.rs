use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{info, warn};

use crate::config::Config;
use crate::message::ServerMessage;
use crate::peer_registry::PeerRegistry;
use crate::redis::{self, RedisRoomError};
use crate::session_registry::SessionRegistry;

#[derive(Debug, Clone, Copy)]
pub enum DegradationReason {
    RedisDown,
    RoomManagerDown,
}

impl DegradationReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DegradationReason::RedisDown => "redis",
            DegradationReason::RoomManagerDown => "room_manager",
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
    purging: Arc<AtomicBool>,
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
            purging: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Best-effort check: attempt a cheap Redis command.  Used to decide whether
    /// to attempt Redis-dependent cleanup during disconnect/purge.
    pub async fn is_redis_available(&self) -> bool {
        let mut conn = match self.redis.get_connection_manager().await {
            Ok(c) => c,
            Err(_) => return false,
        };
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            ::redis::cmd("PING").query_async::<String>(&mut conn),
        )
        .await
        .map(|r: Result<String, ::redis::RedisError>| r.is_ok())
        .unwrap_or(false)
    }

    pub fn is_purging(&self) -> bool {
        self.purging.load(Ordering::SeqCst)
    }

    pub fn try_begin_purge(&self) -> bool {
        self.purging
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub async fn run_purge(&self, reason: DegradationReason) {
        if !self.try_begin_purge() {
            return;
        }
        warn!(reason = reason.as_str(), "purge protocol started");

        let rooms = self.peers.snapshot_rooms_with_sfu().await;
        let retry_after_ms: u32 = 5_000;
        let dependency = reason.as_str().to_owned();

        for (room_id, _participants) in &rooms {
            self.peers
                .broadcast_to_room(
                    room_id,
                    None,
                    ServerMessage::ServiceUnavailable {
                        dependency: dependency.clone(),
                        retry_after_ms,
                    },
                )
                .await;
        }

        for (room_id, participants) in &rooms {
            for (nickname, sfu_addr) in participants {
                if let Some(sfu_addr) = sfu_addr {
                    if let Err(err) = self
                        .sfu
                        .delete_peer(sfu_addr, room_id, nickname, "signaling_purge")
                        .await
                    {
                        warn!(
                            error = %err,
                            sfu_addr = %sfu_addr,
                            room_id = %room_id,
                            nickname = %nickname,
                            "purge: failed best-effort SFU delete_peer"
                        );
                    }
                }
            }
        }

        self.peers.clear().await;
        self.sessions.clear().await;
        self.sfu.clear().await;

        info!(reason = reason.as_str(), "purge protocol completed");
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
                self.sfu.cleanup_local_peer_state(&room_id, &nickname).await;
            }

            for nickname in stale {
                self.peers.unregister(&room_id, &nickname).await;
                self.sfu.cleanup_local_peer_state(&room_id, &nickname).await;
            }

            let affected_sessions = self.sessions.clear_room(&room_id).await;

            if self.is_redis_available().await {
                if let Err(err) = self.room_manager.close_room(&room_id).await {
                    warn!(error = %err, room_id = %room_id, "failed to close room after sfu down");
                }
                if let Err(err) = redis::delete_room(&self.redis, &room_id).await {
                    warn!(
                        error = %err,
                        room_id = %room_id,
                        "failed to delete signaling room members after sfu down"
                    );
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
}

