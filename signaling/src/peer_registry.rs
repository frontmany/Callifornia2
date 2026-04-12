use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::RwLock;

use crate::message::ServerMessage;

type RoomPeers = HashMap<String, HashMap<String, UnboundedSender<ServerMessage>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Delivered,
    Missing,
    Stale,
}

#[derive(Clone, Default)]
pub struct PeerRegistry {
    inner: Arc<RwLock<RoomPeers>>,
}

impl PeerRegistry {
    pub async fn register(&self, room_id: &str, nickname: &str, sender: UnboundedSender<ServerMessage>) {
        let mut peers = self.inner.write().await;
        peers
            .entry(room_id.to_owned())
            .or_default()
            .insert(nickname.to_owned(), sender);
    }

    pub async fn unregister(&self, room_id: &str, nickname: &str) {
        let mut peers = self.inner.write().await;
        let Some(room_peers) = peers.get_mut(room_id) else {
            return;
        };

        room_peers.remove(nickname);
        if room_peers.is_empty() {
            peers.remove(room_id);
        }
    }

    pub async fn send_to_peer(&self, room_id: &str, nickname: &str, payload: ServerMessage) -> DeliveryStatus {
        let sender = {
            let peers = self.inner.read().await;
            peers
                .get(room_id)
                .and_then(|room_peers| room_peers.get(nickname))
                .cloned()
        };

        let Some(sender) = sender else {
            return DeliveryStatus::Missing;
        };

        if sender.send(payload).is_err() {
            return DeliveryStatus::Stale;
        }

        DeliveryStatus::Delivered
    }

    pub async fn broadcast_to_room(
        &self,
        room_id: &str,
        except_nickname: Option<&str>,
        payload: ServerMessage,
    ) -> Vec<String> {
        let targets = {
            let peers = self.inner.read().await;
            let Some(room_peers) = peers.get(room_id) else {
                return Vec::new();
            };

            room_peers
                .iter()
                .filter(|(nickname, _)| except_nickname != Some(nickname.as_str()))
                .map(|(nickname, sender)| (nickname.clone(), sender.clone()))
                .collect::<Vec<_>>()
        };

        let mut stale_nicknames = Vec::new();
        for (nickname, sender) in targets {
            if sender.send(payload.clone()).is_err() {
                stale_nicknames.push(nickname);
            }
        }

        stale_nicknames
    }
}
