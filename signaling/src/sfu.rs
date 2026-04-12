pub mod pb {
    tonic::include_proto!("sfu");
}

use std::time::Duration;

use pb::sfu_event::Event;
use pb::sfu_service_client::SfuServiceClient;
use pb::{
    AddIceCandidateRequest, CreatePeerRequest, DeletePeerRequest, HandleSdpRequest, SdpType,
    SubscribeEventsRequest, SfuEvent,
};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};

use crate::app_state::AppState;
use crate::message::{ServerErrorCode, ServerMessage};
use crate::peer_registry::DeliveryStatus;

const SFU_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct RemoteSdp {
    pub sdp: String,
    pub sdp_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SfuClientError {
    #[error("SFU request failed")]
    Transport(#[from] tonic::Status),
    #[error("{0}")]
    Rejected(String),
}

#[derive(Clone)]
pub struct Client {
    channel: Channel,
    signaling_id: String,
}

impl Client {
    pub fn new(channel: Channel, signaling_id: String) -> Self {
        Self {
            channel,
            signaling_id,
        }
    }

    pub async fn ensure_peer(&self, room_id: &str, nickname: &str) -> Result<(), SfuClientError> {
        let response = self
            .grpc_client()
            .create_peer(CreatePeerRequest {
                peer_id: peer_id(room_id, nickname),
                room_id: room_id.to_owned(),
                signaling_id: self.signaling_id.clone(),
            })
            .await?
            .into_inner();

        ensure_success(response.success, response.error_message, "SFU failed to create peer")
    }

    pub async fn delete_peer(
        &self,
        room_id: &str,
        nickname: &str,
        reason: &str,
    ) -> Result<(), SfuClientError> {
        let response = self
            .grpc_client()
            .delete_peer(DeletePeerRequest {
                peer_id: peer_id(room_id, nickname),
                reason: reason.to_owned(),
            })
            .await?
            .into_inner();

        ensure_success(response.success, response.error_message, "SFU failed to delete peer")
    }

    pub async fn handle_sdp(
        &self,
        room_id: &str,
        nickname: &str,
        sdp: String,
        sdp_type: &str,
    ) -> Result<RemoteSdp, SfuClientError> {
        let response = self
            .grpc_client()
            .handle_sdp(HandleSdpRequest {
                peer_id: peer_id(room_id, nickname),
                sdp,
                r#type: parse_sdp_type(sdp_type) as i32,
            })
            .await?
            .into_inner();

        ensure_success(
            response.success,
            response.error_message.clone(),
            "SFU rejected SDP",
        )?;

        Ok(RemoteSdp {
            sdp: response.sdp,
            sdp_type: format_sdp_type(response.r#type),
        })
    }

    pub async fn add_ice_candidate(
        &self,
        room_id: &str,
        nickname: &str,
        candidate: String,
        sdp_mid: String,
    ) -> Result<(), SfuClientError> {
        let response = self
            .grpc_client()
            .add_ice_candidate(AddIceCandidateRequest {
                peer_id: peer_id(room_id, nickname),
                candidate,
                sdp_mid,
            })
            .await?
            .into_inner();

        ensure_success(
            response.success,
            response.error_message,
            "SFU rejected ICE candidate",
        )
    }

    pub async fn run_event_listener(&self, state: AppState) {
        loop {
            match self.subscribe_events_once(&state).await {
                Ok(()) => warn!("SFU event stream closed, reconnecting"),
                Err(err) => error!(error = %err, "SFU event stream failed"),
            }

            tokio::time::sleep(SFU_RECONNECT_DELAY).await;
        }
    }

    fn grpc_client(&self) -> SfuServiceClient<Channel> {
        SfuServiceClient::new(self.channel.clone())
    }

    async fn subscribe_events_once(&self, state: &AppState) -> Result<(), tonic::Status> {
        let mut stream = self
            .grpc_client()
            .subscribe_events(SubscribeEventsRequest {
                signaling_id: self.signaling_id.clone(),
            })
            .await?
            .into_inner();

        info!(signaling_id = %self.signaling_id, "Subscribed to SFU events");

        while let Some(event) = stream.message().await? {
            handle_sfu_event(state, event).await;
        }

        Ok(())
    }
}

fn ensure_success(
    success: bool,
    error_message: Option<String>,
    fallback: &str,
) -> Result<(), SfuClientError> {
    if success {
        return Ok(());
    }

    Err(SfuClientError::Rejected(
        error_message.unwrap_or_else(|| fallback.to_owned()),
    ))
}

async fn handle_sfu_event(state: &AppState, event: SfuEvent) {
    let SfuEvent {
        peer_id,
        room_id,
        event,
    } = event;
    let (_, nickname_from_peer_id) = split_peer_id(&peer_id);

    match event {
        Some(Event::PeerConnected(connected)) => {
            info!(
                peer_id = %peer_id,
                room_id = %room_id,
                transport = %connected.transport_type,
                "SFU peer connected"
            );
        }
        Some(Event::PeerDisconnected(disconnected)) => {
            warn!(
                peer_id = %peer_id,
                room_id = %room_id,
                reason = %disconnected.reason,
                "SFU peer disconnected"
            );
            if let Some(nickname) = nickname_from_peer_id {
                broadcast_to_room(
                    state,
                    &room_id,
                    Some(&nickname),
                    ServerMessage::ParticipantLeft {
                        nickname: nickname.clone(),
                    },
                )
                .await;
            }
        }
        Some(Event::TrackAdded(track)) => {
            info!(
                peer_id = %peer_id,
                room_id = %room_id,
                track_id = %track.track_id,
                kind = %track.kind,
                codec = %track.codec,
                "SFU track added"
            );
        }
        Some(Event::TrackRemoved(track)) => {
            info!(
                peer_id = %peer_id,
                room_id = %room_id,
                track_id = %track.track_id,
                reason = %track.reason,
                "SFU track removed"
            );
        }
        Some(Event::IceCandidate(candidate)) => {
            debug!(
                peer_id = %peer_id,
                room_id = %room_id,
                sdp_mid = %candidate.sdp_mid,
                "SFU emitted ICE candidate event"
            );
            send_to_peer(
                state,
                &room_id,
                &peer_id,
                ServerMessage::Candidate {
                    from: "sfu".to_owned(),
                    candidate: candidate.candidate,
                    sdp_mid: candidate.sdp_mid,
                },
            )
            .await;
        }
        Some(Event::Error(err)) => {
            error!(
                peer_id = %peer_id,
                room_id = %room_id,
                code = %err.error_code,
                fatal = err.is_fatal,
                message = %err.error_message,
                "SFU reported error"
            );
            send_to_peer(
                state,
                &room_id,
                &peer_id,
                ServerMessage::Error {
                    code: ServerErrorCode::SfuUnavailable,
                    message: err.error_message,
                },
            )
            .await;
        }
        Some(Event::Heartbeat(heartbeat)) => {
            debug!(
                active_peers = heartbeat.active_peers,
                active_rooms = heartbeat.active_rooms,
                timestamp = heartbeat.timestamp,
                "SFU heartbeat"
            );
        }
        None => {
            debug!(peer_id = %peer_id, room_id = %room_id, "Received empty SFU event");
        }
    }
}

async fn send_to_peer(state: &AppState, room_id: &str, peer_id: &str, payload: ServerMessage) {
    let (_, Some(nickname)) = split_peer_id(peer_id) else {
        warn!(peer_id = %peer_id, "unable to route SFU event: invalid peer_id format");
        return;
    };

    match state.peers.send_to_peer(room_id, &nickname, payload).await {
        DeliveryStatus::Delivered => {}
        DeliveryStatus::Missing => {
            debug!(room_id = %room_id, nickname = %nickname, "peer not connected locally");
        }
        DeliveryStatus::Stale => {
            state.detach_peer(room_id, &nickname, "stale_sender").await;
            let stale = state
                .peers
                .broadcast_to_room(
                    room_id,
                    Some(&nickname),
                    ServerMessage::ParticipantLeft {
                        nickname: nickname.clone(),
                    },
                )
                .await;
            
            for stale_nickname in stale {
                state
                    .detach_peer(room_id, &stale_nickname, "stale_sender")
                    .await;
            }
        }
    }
}

async fn broadcast_to_room(
    state: &AppState,
    room_id: &str,
    except_nickname: Option<&str>,
    payload: ServerMessage,
) {
    let stale = state
        .peers
        .broadcast_to_room(room_id, except_nickname, payload)
        .await;

    for nickname in stale {
        state.detach_peer(room_id, &nickname, "stale_sender").await;
    }
}

fn split_peer_id(peer_id: &str) -> (Option<String>, Option<String>) {
    match peer_id.split_once(':') {
        Some((room_id, nickname)) => (Some(room_id.to_owned()), Some(nickname.to_owned())),
        None => (None, None),
    }
}

fn parse_sdp_type(value: &str) -> SdpType {
    match value {
        "offer" => SdpType::Offer,
        "answer" => SdpType::Answer,
        _ => SdpType::Unspecified,
    }
}

fn format_sdp_type(value: i32) -> String {
    match SdpType::try_from(value).unwrap_or(SdpType::Unspecified) {
        SdpType::Offer => "offer".to_owned(),
        SdpType::Answer => "answer".to_owned(),
        SdpType::Unspecified => "answer".to_owned(),
    }
}

fn peer_id(room_id: &str, nickname: &str) -> String {
    format!("{room_id}:{nickname}")
}