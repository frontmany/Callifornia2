use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerErrorCode {
    InvalidJson,
    InvalidPayload,
    RoomNotFound,
    NicknameTaken,
    Unauthorized,
    SessionConflict,
    AlreadyAuthorized,
    LeaveRoomMismatch,
    AlreadyInRoom,
    NotInRoom,
    RoomNotReady,
    TransferUnavailable,
    SfuUnavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Attach {
        token: String,
    },
    Logout {
        session_id: String,
        participants: Option<Vec<String>>,
    },
    Sdp {
        session_id: String,
        sdp: String,
        sdp_type: String,
    },
    Candidate {
        session_id: String,
        candidate: String,
        sdp_mid: String,
    },
    Create {
        session_id: String,
    },
    Join {
        session_id: String,
        room_id: String,
    },
    Leave {
        session_id: String,
        room_id: String,
        participants: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Attached {
        nickname: String,
        session_id: String,
    },
    LoggedOut {
        nickname: String,
    },
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
    TransferRequired {
        room_id: String,
        target_host: String,
        target_port: u16,
    },
    RoomClosed {
        room_id: String,
        reason: String,
    },
    Left {
        room_id: String,
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
    #[error("session_id must be a valid UUID")]
    InvalidSessionId,
    #[error("room_id must be a valid UUID")]
    InvalidRoomId,
    #[error("token must not be empty")]
    InvalidToken,
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
            ClientMessage::Attach { token } => validate_token(token),
            ClientMessage::Logout { session_id, .. } => validate_session_id(session_id),
            ClientMessage::Create { session_id } => validate_session_id(session_id),
            ClientMessage::Join {
                session_id,
                room_id,
            } => {
                validate_session_id(session_id)?;
                validate_room_id(room_id)
            }
            ClientMessage::Leave {
                session_id,
                room_id,
                ..
            } => {
                validate_session_id(session_id)?;
                validate_room_id(room_id)
            }
            ClientMessage::Sdp {
                session_id, sdp, ..
            } => {
                validate_session_id(session_id)?;
                validate_sdp(sdp)
            }
            ClientMessage::Candidate {
                session_id,
                candidate,
                sdp_mid,
            } => {
                validate_session_id(session_id)?;
                validate_candidate(candidate, sdp_mid)
            }
        }
    }
}

fn validate_token(token: &str) -> Result<(), ValidationError> {
    if token.trim().is_empty() {
        return Err(ValidationError::InvalidToken);
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
