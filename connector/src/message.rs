use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Auth { nickname: String },
    Create,
    Join { room_id: String },
    Logout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    AuthOk {
        nickname: String,
        session_id: String,
    },
    SignalingReady {
        signaling_url: String,
        session_id: String,
        token: String,
    },
    LoggedOut,
    Error {
        code: ServerErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerErrorCode {
    InvalidJson,
    InvalidPayload,
    Unauthorized,
    NicknameTaken,
    RoomNotFound,
    Internal,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("nickname must be 3..32 characters and contain only letters, digits, '_' or '-'")]
    InvalidNickname,
    #[error("room_id must be a valid UUID")]
    InvalidRoomId,
}

impl ClientMessage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            ClientMessage::Auth { nickname } => validate_nickname(nickname),
            ClientMessage::Join { room_id } => validate_room_id(room_id),
            ClientMessage::Create | ClientMessage::Logout => Ok(()),
        }
    }
}

fn validate_nickname(nickname: &str) -> Result<(), ValidationError> {
    let trimmed = nickname.trim();
    let len = trimmed.chars().count();
    if !(3..=32).contains(&len) {
        return Err(ValidationError::InvalidNickname);
    }

    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(ValidationError::InvalidNickname);
    }

    Ok(())
}

fn validate_room_id(room_id: &str) -> Result<(), ValidationError> {
    Uuid::parse_str(room_id)
        .map(|_| ())
        .map_err(|_| ValidationError::InvalidRoomId)
}
