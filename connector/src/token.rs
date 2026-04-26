use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorClaims {
    pub session_id: String,
    pub nickname: String,
    pub signaling_instance_id: String,
    pub intent: String,
    pub room_id: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

pub fn issue_token(
    secret: &str,
    ttl: Duration,
    session_id: String,
    nickname: String,
    signaling_instance_id: String,
    intent: String,
    room_id: Option<String>,
) -> Result<String, jsonwebtoken::errors::Error> {
    let iat = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = ConnectorClaims {
        session_id,
        nickname,
        signaling_instance_id,
        intent,
        room_id,
        iat,
        exp: iat + ttl.as_secs(),
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}
