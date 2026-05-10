// ── Signaling namespace ──────────────────────────────────────────────────────
pub const SIGNALING_NODES_KEY: &str = "signaling:nodes";
pub const SIGNALING_INSTANCE_LOAD_KEY: &str = "signaling:instance_load";
pub const SIGNALING_SESSION_PREFIX: &str = "signaling:session:";
pub const SIGNALING_NICK_PREFIX: &str = "signaling:nick:";
pub const SIGNALING_ROOM_PREFIX: &str = "signaling:room:";
pub const SIGNALING_JTI_PREFIX: &str = "signaling:jti:";

// ── Room-manager namespace ────────────────────────────────────────────────────
pub const ROOM_BINDING_PREFIX: &str = "room_manager:room:";
pub const SFU_INSTANCES_KEY: &str = "room_manager:sfu_instances";
pub const SFU_ROOM_LOAD_KEY: &str = "room_manager:sfu_room_load";
pub const WAITING_REQUESTS_KEY: &str = "room_manager:waiting_requests";

// ── Supervisor namespace ──────────────────────────────────────────────────────
pub const SUPERVISOR_HEARTBEAT_KEY: &str = "supervisor:heartbeat";

// ── Key builders ─────────────────────────────────────────────────────────────

pub fn session_key(session_id: &str) -> String {
    format!("{SIGNALING_SESSION_PREFIX}{session_id}")
}

pub fn session_lock_key(session_id: &str) -> String {
    format!("{SIGNALING_SESSION_PREFIX}{session_id}:lock")
}

pub fn nick_key(nickname: &str) -> String {
    format!("{SIGNALING_NICK_PREFIX}{nickname}")
}

pub fn room_members_key(room_id: &str) -> String {
    format!("{SIGNALING_ROOM_PREFIX}{room_id}:members")
}

pub fn room_binding_key(room_id: &str) -> String {
    format!("{ROOM_BINDING_PREFIX}{room_id}:binding")
}
