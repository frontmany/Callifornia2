use serde::{Deserialize, Serialize};

// ── SFU models ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfuInstance {
    pub instance_id: String,
    pub grpc_addr: String,
    pub max_rooms: u32,
    pub alive: bool,
    pub state: String,
    pub last_ping_unix: i64,
    pub idle_since_unix: i64,
}

#[derive(Debug, Clone)]
pub struct SfuCandidate {
    pub instance_id: String,
    pub grpc_addr: String,
    pub max_rooms: u32,
}

// ── Room models ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RoomBinding {
    pub room_id: String,
    pub owner_id: String,
    pub owner_host: String,
    pub owner_port: u16,
    pub state: String,
    pub sfu_instance_id: Option<String>,
    pub sfu_grpc_addr: Option<String>,
}

// ── Supervisor status ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SupervisorStatus {
    pub last_seen_unix: i64,
    pub instance_id: Option<String>,
}
