use crate::connector_api::SignalingReadyResponse;
use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CloseEvent, ErrorEvent, Event, MessageEvent, WebSocket};
use yew::Callback;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignalingIntent {
    Create,
    Join { room_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalingStart {
    pub signaling_url: String,
    pub session_id: String,
    pub token: String,
    pub intent: SignalingIntent,
}

impl SignalingStart {
    pub fn new(response: SignalingReadyResponse, intent: SignalingIntent) -> Self {
        Self {
            signaling_url: response.signaling_url,
            session_id: response.session_id,
            token: response.token,
            intent,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomSnapshot {
    pub room_id: String,
    pub your_nickname: String,
    pub participants: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum SignalingEvent {
    Connected,
    Disconnected {
        reason: Option<String>,
    },
    RoomReady(RoomSnapshot),
    ParticipantJoined(String),
    ParticipantLeft(String),
    Left {
        room_id: String,
    },
    RoomClosed {
        room_id: String,
        reason: String,
    },
    Error(SignalingError),
    ServiceUnavailable {
        dependency: String,
        retry_after_ms: u32,
    },
    RemoteSdp {
        sdp: String,
        sdp_type: String,
    },
    RemoteIceCandidate {
        candidate: String,
        sdp_mid: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalingError {
    pub code: Option<ServerErrorCode>,
    pub message: String,
}

impl SignalingError {
    fn local(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    fn server(code: ServerErrorCode, message: String) -> Self {
        Self {
            code: Some(code),
            message: format!("{code:?}: {message}"),
        }
    }
}

/// Lightweight, clonable handle for sending SDP and ICE messages over the open WebSocket.
///
/// `WebSocket` in web-sys is a JS reference type whose `Clone` just increments the
/// JS reference count, so all clones share the same underlying connection.
#[derive(Clone)]
pub struct SignalingTransport {
    ws: WebSocket,
    session_id: String,
}

impl SignalingTransport {
    pub fn send_sdp(&self, sdp: String, sdp_type: String) -> Result<(), String> {
        send_client_message(
            &self.ws,
            &ClientMessage::Sdp {
                session_id: self.session_id.clone(),
                sdp,
                sdp_type,
            },
        )
    }

    pub fn send_ice_candidate(&self, candidate: String, sdp_mid: String) -> Result<(), String> {
        send_client_message(
            &self.ws,
            &ClientMessage::Candidate {
                session_id: self.session_id.clone(),
                candidate,
                sdp_mid,
            },
        )
    }
}

pub struct SignalingClient {
    ws: WebSocket,
    session_id: String,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(ErrorEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl SignalingClient {
    pub fn connect(
        start: SignalingStart,
        events: Callback<SignalingEvent>,
    ) -> Result<Self, String> {
        let ws = WebSocket::new(&start.signaling_url).map_err(js_error_to_string)?;

        let on_open = {
            let ws = ws.clone();
            let token = start.token.clone();
            let events = events.clone();
            Closure::wrap(Box::new(move |_| {
                events.emit(SignalingEvent::Connected);
                if let Err(err) = send_client_message(
                    &ws,
                    &ClientMessage::Attach {
                        token: token.clone(),
                    },
                ) {
                    events.emit(SignalingEvent::Error(SignalingError::local(err)));
                }
            }) as Box<dyn FnMut(Event)>)
        };

        let on_message = {
            let ws = ws.clone();
            let start = start.clone();
            let events = events.clone();
            Closure::wrap(Box::new(move |event: MessageEvent| {
                let Some(text) = event.data().as_string() else {
                    events.emit(SignalingEvent::Error(SignalingError::local(
                        "Unsupported binary signaling message.".to_owned(),
                    )));
                    return;
                };

                let message = match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => message,
                    Err(err) => {
                        events.emit(SignalingEvent::Error(SignalingError::local(format!(
                            "Invalid signaling message: {err}"
                        ))));
                        return;
                    }
                };

                handle_server_message(&ws, &start, &events, message);
            }) as Box<dyn FnMut(MessageEvent)>)
        };

        let on_error = {
            let events = events.clone();
            Closure::wrap(Box::new(move |_| {
                events.emit(SignalingEvent::Error(SignalingError::local(
                    "Signaling WebSocket error.".to_owned(),
                )));
            }) as Box<dyn FnMut(ErrorEvent)>)
        };

        let on_close = {
            let events = events.clone();
            Closure::wrap(Box::new(move |event: CloseEvent| {
                let reason = if event.reason().is_empty() {
                    None
                } else {
                    Some(event.reason())
                };
                events.emit(SignalingEvent::Disconnected { reason });
            }) as Box<dyn FnMut(CloseEvent)>)
        };

        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        Ok(Self {
            ws,
            session_id: start.session_id.clone(),
            _on_open: on_open,
            _on_message: on_message,
            _on_error: on_error,
            _on_close: on_close,
        })
    }

    /// Return a clonable transport handle that can send SDP/ICE messages independently.
    pub fn transport(&self) -> SignalingTransport {
        SignalingTransport {
            ws: self.ws.clone(),
            session_id: self.session_id.clone(),
        }
    }

    pub fn leave(
        &self,
        session_id: &str,
        room_id: &str,
        participants: &[String],
    ) -> Result<(), String> {
        send_client_message(
            &self.ws,
            &ClientMessage::Leave {
                session_id: session_id.to_owned(),
                room_id: room_id.to_owned(),
                participants: Some(participants.to_vec()),
            },
        )
    }
}

impl Drop for SignalingClient {
    fn drop(&mut self) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onerror(None);
        self.ws.set_onclose(None);
        let _ = self.ws.close();
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Attach {
        token: String,
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
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Attached {
        nickname: String,
        session_id: String,
    },
    LoggedOut {
        nickname: String,
    },
    Sdp {
        #[serde(rename = "from")]
        _from: String,
        sdp: String,
        sdp_type: String,
    },
    Candidate {
        #[serde(rename = "from")]
        _from: String,
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
    ServiceUnavailable {
        dependency: String,
        retry_after_ms: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerErrorCode {
    InvalidJson,
    InvalidPayload,
    Unauthorized,
    SessionConflict,
    AlreadyAuthorized,
    LeaveRoomMismatch,
    RoomNotFound,
    NicknameTaken,
    AlreadyInRoom,
    NotInRoom,
    SfuCapacityExhausted,
    SfuUnavailable,
    SfuRejected,
    InvalidRoomAssignment,
    StorageUnavailable,
    WriteFailed,
}

fn handle_server_message(
    ws: &WebSocket,
    start: &SignalingStart,
    events: &Callback<SignalingEvent>,
    message: ServerMessage,
) {
    match message {
        ServerMessage::Attached {
            nickname: _nickname,
            session_id,
        } => {
            if session_id != start.session_id {
                events.emit(SignalingEvent::Error(SignalingError::local(
                    "Signaling attached an unexpected session.".to_owned(),
                )));
                return;
            }
            let next = match &start.intent {
                SignalingIntent::Create => ClientMessage::Create {
                    session_id: start.session_id.clone(),
                },
                SignalingIntent::Join { room_id } => ClientMessage::Join {
                    session_id: start.session_id.clone(),
                    room_id: room_id.clone(),
                },
            };
            if let Err(err) = send_client_message(ws, &next) {
                events.emit(SignalingEvent::Error(SignalingError::local(err)));
            }
        }
        ServerMessage::Created {
            room_id,
            your_nickname,
        } => {
            events.emit(SignalingEvent::RoomReady(RoomSnapshot {
                room_id,
                participants: vec![your_nickname.clone()],
                your_nickname,
            }));
        }
        ServerMessage::Joined {
            room_id,
            your_nickname,
            mut participants,
        } => {
            if !participants.iter().any(|nick| nick == &your_nickname) {
                participants.push(your_nickname.clone());
            }
            events.emit(SignalingEvent::RoomReady(RoomSnapshot {
                room_id,
                your_nickname,
                participants,
            }));
        }
        ServerMessage::ParticipantJoined { nickname } => {
            events.emit(SignalingEvent::ParticipantJoined(nickname));
        }
        ServerMessage::ParticipantLeft { nickname } => {
            events.emit(SignalingEvent::ParticipantLeft(nickname));
        }
        ServerMessage::Left { room_id } => {
            events.emit(SignalingEvent::Left { room_id });
        }
        ServerMessage::RoomClosed { room_id, reason } => {
            events.emit(SignalingEvent::RoomClosed { room_id, reason });
        }
        ServerMessage::Error { code, message } => {
            events.emit(SignalingEvent::Error(SignalingError::server(code, message)));
        }
        ServerMessage::ServiceUnavailable {
            dependency,
            retry_after_ms,
        } => {
            events.emit(SignalingEvent::ServiceUnavailable {
                dependency,
                retry_after_ms,
            });
        }
        ServerMessage::LoggedOut { nickname } => {
            events.emit(SignalingEvent::Error(SignalingError::local(format!(
                "{nickname} logged out."
            ))));
        }
        ServerMessage::Sdp { sdp, sdp_type, .. } => {
            events.emit(SignalingEvent::RemoteSdp { sdp, sdp_type });
        }
        ServerMessage::Candidate { candidate, sdp_mid, .. } => {
            events.emit(SignalingEvent::RemoteIceCandidate { candidate, sdp_mid });
        }
    }
}

fn send_client_message(ws: &WebSocket, message: &ClientMessage) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|err| err.to_string())?;
    ws.send_with_str(&text).map_err(|err| {
        js_error_message(err).unwrap_or_else(|| "Failed to send signaling message.".to_owned())
    })
}

fn js_error_message(value: JsValue) -> Option<String> {
    value.as_string().or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|s| s.as_string())
    })
}

fn js_error_to_string(value: JsValue) -> String {
    js_error_message(value).unwrap_or_else(|| "JavaScript operation failed.".to_owned())
}
