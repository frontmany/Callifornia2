use serde::{Deserialize, Serialize};
use uuid::Uuid;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerErrorCode { 
    InvalidJson,
    InvalidPayload,
    NotImplemented,
    RoomNotFound,
    NicknameTaken,
    NotInRoom,
    SfuUnavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Sdp {
        sdp: String,
        sdp_type: String,
    },
    Candidate {
        candidate: String,
        sdp_mid: String,
    },
    Create {
        nickname: String,
    },
    Join {
        room_id: String,
        nickname: String,
    },
    Leave {
        room_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Sdp {
        from: String,
        sdp: String,
        sdp_type: String,
    },
    Candidate {
        from: String,
        candidate: String,
        sdp_mid: String,
    },
    Created {
        room_id: String,
        your_nickname: String,
    },
    Joined {
        room_id: String,
        your_nickname: String,
        participants: Vec<String>,
    },
    ParticipantJoined {
        nickname: String,
    },
    ParticipantLeft {
        nickname: String,
    },
    Error {
        code: ServerErrorCode,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("nickname must be 3..32 characters and contain only letters, digits, '_' or '-'")]
    InvalidNickname,
    #[error("room_id must be a valid UUID")]
    InvalidRoomId,
    #[error("sdp must not be empty")]
    MissingSdp,
    #[error("candidate must not be empty")]
    MissingCandidate,
    #[error("sdp_mid must not be empty")]
    MissingSdpMid,
}

impl ClientMessage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            ClientMessage::Create { nickname } => validate_nickname(nickname),
            ClientMessage::Join { room_id, nickname } => {
                validate_room_id(room_id)?;
                validate_nickname(nickname)
            }
            ClientMessage::Leave { room_id } => validate_room_id(room_id),
            ClientMessage::Sdp { sdp, .. } => validate_sdp(sdp),
            ClientMessage::Candidate { candidate, sdp_mid } => {
                validate_candidate(candidate, sdp_mid)
            }
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

fn validate_sdp(sdp: &str) -> Result<(), ValidationError> {
    if sdp.trim().is_empty() {
        return Err(ValidationError::MissingSdp);
    }
    Ok(())
}

fn validate_candidate(candidate: &str, sdp_mid: &str) -> Result<(), ValidationError> {
    if candidate.trim().is_empty() {
        return Err(ValidationError::MissingCandidate);
    }
    if sdp_mid.trim().is_empty() {
        return Err(ValidationError::MissingSdpMid);
    }
    Ok(())
}