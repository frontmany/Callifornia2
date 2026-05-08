use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub nickname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub nickname: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRequest {
    pub session_id: String,
    pub room_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogoutRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalingReadyResponse {
    pub signaling_url: String,
    pub session_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: ServerErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerErrorCode {
    InvalidPayload,
    Unauthorized,
    NicknameTaken,
    RoomNotFound,
    StorageUnavailable,
    NoHealthySignaling,
    UnknownSignalingRoute,
    TokenIssueFailed,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("nickname must be 3..32 characters and contain only letters, digits, '_' or '-'")]
    InvalidNickname,
    #[error("session_id must be a valid UUID")]
    InvalidSessionId,
    #[error("room_id must be a valid UUID")]
    InvalidRoomId,
}

impl AuthRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_nickname(&self.nickname)
    }
}

impl CreateRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_session_id(&self.session_id)
    }
}

impl JoinRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_session_id(&self.session_id)?;
        validate_room_id(&self.room_id)
    }
}

impl LogoutRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_session_id(&self.session_id)
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

fn validate_session_id(session_id: &str) -> Result<(), ValidationError> {
    Uuid::parse_str(session_id)
        .map(|_| ())
        .map_err(|_| ValidationError::InvalidSessionId)
}
