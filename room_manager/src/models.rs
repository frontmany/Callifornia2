use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfuCandidate {
    pub instance_id: String,
    pub grpc_addr: String,
    pub max_clients: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfuInstanceRecord {
    pub instance_id: String,
    pub grpc_addr: String,
    pub max_clients: u32,
    pub alive: bool,
    pub provisioned: bool,
    pub state: String,
    pub last_ping_unix: i64,
    pub idle_since_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitingRequestRecord {
    pub room_id: String,
    pub signaling_owner_id: String,
    pub signaling_owner_host: String,
    pub signaling_owner_port: u16,
    pub enqueued_at_unix: i64,
}

#[derive(Debug, Clone)]
pub struct RoomAssignment {
    pub room_id: String,
    pub owner_id: String,
    pub owner_host: String,
    pub owner_port: u16,
    pub state: String,
    pub sfu_instance_id: Option<String>,
    pub sfu_grpc_addr: Option<String>,
}

#[derive(Clone)]
pub struct StaticInventoryProvider {
    available: Arc<Mutex<VecDeque<SfuCandidate>>>,
    provisioned: Arc<Mutex<HashSet<String>>>,
}

impl StaticInventoryProvider {
    pub fn new(candidates: Vec<SfuCandidate>) -> Self {
        Self {
            available: Arc::new(Mutex::new(VecDeque::from(candidates))),
            provisioned: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn provision_next(&self) -> Option<SfuCandidate> {
        let mut queue = self.available.lock().await;
        let candidate = queue.pop_front()?;
        self.provisioned
            .lock()
            .await
            .insert(candidate.instance_id.clone());
        Some(candidate)
    }

    pub async fn release(&self, instance_id: &str, candidate: SfuCandidate) {
        let mut provisioned = self.provisioned.lock().await;
        if !provisioned.remove(instance_id) {
            return;
        }
        drop(provisioned);

        self.available.lock().await.push_back(candidate);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub redis: redis::Client,
    pub provider: StaticInventoryProvider,
}

#[derive(Clone)]
pub struct RoomManagerServer {
    pub state: AppState,
}
