pub mod pb {
    tonic::include_proto!("sfu");
}

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use pb::sfu_event::Event;
use pb::sfu_service_client::SfuServiceClient;
use pb::{
    AddIceCandidateRequest, CreatePeerRequest, DeletePeerRequest, HandleSdpRequest, SdpType,
    SfuEvent, SubscribeEventsRequest,
};
use rand::Rng;
use tokio::sync::RwLock;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, error, info, warn};

use crate::message::{ServerErrorCode, ServerMessage};
use crate::peer_registry::DeliveryStatus;
use crate::state::State;

#[derive(Debug, Clone)]
pub struct RemoteSdp {
    pub sdp: String,
    pub sdp_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SfuClientError {
    #[error("SFU request failed")]
    Transport(#[from] tonic::Status),
    #[error("failed to connect to SFU endpoint")]
    Connect(#[from] tonic::transport::Error),
    #[error("SFU request timed out")]
    Timeout,
    #[error("{0}")]
    Rejected(String),
}

#[derive(Clone)]
pub struct Registry {
    signaling_id: String,
    connect_timeout: Duration,
    rpc_timeout: Duration,
    backoff_min: Duration,
    backoff_max: Duration,
    clients: Arc<RwLock<HashMap<String, Channel>>>,
    listeners: Arc<RwLock<HashSet<String>>>,
}

impl Registry {
    pub fn new(
        signaling_id: String,
        connect_timeout: Duration,
        rpc_timeout: Duration,
        backoff_min: Duration,
        backoff_max: Duration,
    ) -> Self {
        Self {
            signaling_id,
            connect_timeout,
            rpc_timeout,
            backoff_min,
            backoff_max,
            clients: Arc::new(RwLock::new(HashMap::new())),
            listeners: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    async fn invalidate_channel(&self, sfu_addr: &str) {
        self.clients.write().await.remove(sfu_addr);
    }

    async fn run_rpc<T, F, Fut>(
        &self,
        sfu_addr: &str,
        op: F,
    ) -> Result<T, SfuClientError>
    where
        F: FnOnce(SfuServiceClient<Channel>) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Response<T>, tonic::Status>>,
    {
        let client = self.grpc_client(sfu_addr).await?;
        match tokio::time::timeout(self.rpc_timeout, op(client)).await {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(status)) => {
                if matches!(
                    status.code(),
                    tonic::Code::Unavailable | tonic::Code::Cancelled | tonic::Code::DeadlineExceeded
                ) {
                    self.invalidate_channel(sfu_addr).await;
                }
                Err(SfuClientError::Transport(status))
            }
            Err(_) => {
                self.invalidate_channel(sfu_addr).await;
                Err(SfuClientError::Timeout)
            }
        }
    }

    pub async fn clear(&self) {
        self.clients.write().await.clear();
        self.listeners.write().await.clear();
    }

    pub async fn ensure_peer(
        &self,
        state: &State,
        sfu_addr: &str,
        room_id: &str,
        nickname: &str,
    ) -> Result<(), SfuClientError> {
        self.ensure_listener(state.clone(), sfu_addr.to_owned())
            .await?;
        let req = CreatePeerRequest {
            room_id: room_id.to_owned(),
            participant_id: nickname.to_owned(),
            signaling_id: self.signaling_id.clone(),
        };
        let response = self
            .run_rpc(sfu_addr, |mut client| async move { client.create_peer(req).await })
            .await?;
        ensure_success(
            response.success,
            response.error_message,
            "SFU failed to create peer",
        )
    }

    pub async fn delete_peer(
        &self,
        sfu_addr: &str,
        room_id: &str,
        nickname: &str,
        reason: &str,
    ) -> Result<(), SfuClientError> {
        let req = DeletePeerRequest {
            room_id: room_id.to_owned(),
            participant_id: nickname.to_owned(),
            reason: reason.to_owned(),
        };
        let response = self
            .run_rpc(sfu_addr, |mut client| async move { client.delete_peer(req).await })
            .await?;
        ensure_success(
            response.success,
            response.error_message,
            "SFU failed to delete peer",
        )
    }

    pub async fn handle_sdp(
        &self,
        sfu_addr: &str,
        room_id: &str,
        nickname: &str,
        sdp: String,
        sdp_type: &str,
    ) -> Result<RemoteSdp, SfuClientError> {
        let req = HandleSdpRequest {
            room_id: room_id.to_owned(),
            participant_id: nickname.to_owned(),
            sdp,
            r#type: parse_sdp_type(sdp_type) as i32,
        };
        let response = self
            .run_rpc(sfu_addr, |mut client| async move { client.handle_sdp(req).await })
            .await?;

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
        sfu_addr: &str,
        room_id: &str,
        nickname: &str,
        candidate: String,
        sdp_mid: String,
    ) -> Result<(), SfuClientError> {
        let req = AddIceCandidateRequest {
            room_id: room_id.to_owned(),
            participant_id: nickname.to_owned(),
            candidate,
            sdp_mid,
        };
        let response = self
            .run_rpc(sfu_addr, |mut client| async move {
                client.add_ice_candidate(req).await
            })
            .await?;

        ensure_success(
            response.success,
            response.error_message,
            "SFU rejected ICE candidate",
        )
    }

    async fn grpc_client(
        &self,
        sfu_addr: &str,
    ) -> Result<SfuServiceClient<Channel>, SfuClientError> {
        let channel = self.channel_for(sfu_addr).await?;
        Ok(SfuServiceClient::new(channel))
    }

    async fn channel_for(&self, sfu_addr: &str) -> Result<Channel, SfuClientError> {
        if let Some(channel) = self.clients.read().await.get(sfu_addr).cloned() {
            return Ok(channel);
        }

        let endpoint = Endpoint::from_shared(sfu_addr.to_owned())?
            .connect_timeout(self.connect_timeout)
            .tcp_nodelay(true);
        let channel = endpoint.connect().await?;
        self.clients
            .write()
            .await
            .insert(sfu_addr.to_owned(), channel.clone());
        Ok(channel)
    }

    async fn ensure_listener(&self, state: State, sfu_addr: String) -> Result<(), SfuClientError> {
        {
            let listeners = self.listeners.read().await;
            if listeners.contains(&sfu_addr) {
                return Ok(());
            }
        }

        self.listeners.write().await.insert(sfu_addr.clone());
        let registry = self.clone();
        tokio::spawn(async move {
            registry.run_event_listener(state, sfu_addr).await;
        });
        Ok(())
    }

    async fn run_event_listener(&self, state: State, sfu_addr: String) {
        let mut delay = self.backoff_min;
        loop {
            match self.subscribe_events_once(&state, &sfu_addr).await {
                Ok(()) => {
                    warn!(sfu_addr = %sfu_addr, "SFU event stream closed, reconnecting");
                    delay = self.backoff_min;
                }
                Err(err) => {
                    error!(error = %err, sfu_addr = %sfu_addr, "SFU event stream failed");
                }
            }

            self.invalidate_channel(&sfu_addr).await;

            state
                .close_rooms_due_to_sfu_addr(&sfu_addr, "sfu connection lost; room closed")
                .await;

            let jitter_ms = rand::thread_rng().gen_range(0..=delay.as_millis() as u64 / 2);
            let total = delay + Duration::from_millis(jitter_ms);
            tokio::time::sleep(total).await;
            delay = std::cmp::min(delay * 2, self.backoff_max);
        }
    }

    async fn subscribe_events_once(
        &self,
        state: &State,
        sfu_addr: &str,
    ) -> Result<(), tonic::Status> {
        let mut stream = self
            .grpc_client(sfu_addr)
            .await
            .map_err(map_connect_error)?
            .subscribe_events(SubscribeEventsRequest {
                signaling_id: self.signaling_id.clone(),
            })
            .await?
            .into_inner();

        info!(signaling_id = %self.signaling_id, sfu_addr = %sfu_addr, "Subscribed to SFU events");

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

fn map_connect_error(err: SfuClientError) -> tonic::Status {
    match err {
        SfuClientError::Transport(status) => status,
        SfuClientError::Connect(err) => tonic::Status::unavailable(err.to_string()),
        SfuClientError::Timeout => tonic::Status::deadline_exceeded("sfu rpc timed out"),
        SfuClientError::Rejected(message) => tonic::Status::failed_precondition(message),
    }
}

async fn handle_sfu_event(state: &State, event: SfuEvent) {
    let SfuEvent {
        room_id,
        participant_id,
        event,
    } = event;

    match event {
        Some(Event::PeerConnected(connected)) => {
            info!(
                participant_id = %participant_id,
                room_id = %room_id,
                transport = %connected.transport_type,
                "SFU peer connected"
            );
        }
        Some(Event::PeerDisconnected(disconnected)) => {
            warn!(
                participant_id = %participant_id,
                room_id = %room_id,
                reason = %disconnected.reason,
                "SFU peer disconnected"
            );
            if !participant_id.is_empty() {
                broadcast_to_room(
                    state,
                    &room_id,
                    Some(participant_id.as_str()),
                    ServerMessage::ParticipantLeft {
                        nickname: participant_id.clone(),
                    },
                )
                .await;
            }
        }
        Some(Event::TrackAdded(track)) => {
            info!(
                participant_id = %participant_id,
                room_id = %room_id,
                track_id = %track.track_id,
                kind = %track.kind,
                codec = %track.codec,
                is_sending = track.is_sending,
                "SFU track added"
            );
        }
        Some(Event::TrackRemoved(track)) => {
            info!(
                participant_id = %participant_id,
                room_id = %room_id,
                track_id = %track.track_id,
                kind = %track.kind,
                reason = %track.reason,
                "SFU track removed"
            );
        }
        Some(Event::IceCandidate(candidate)) => {
            debug!(
                participant_id = %participant_id,
                room_id = %room_id,
                sdp_mid = %candidate.sdp_mid,
                "SFU emitted ICE candidate event"
            );
            send_to_peer(
                state,
                &room_id,
                &participant_id,
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
                participant_id = %participant_id,
                room_id = %room_id,
                code = %err.error_code,
                fatal = err.is_fatal,
                message = %err.error_message,
                "SFU reported error"
            );
            send_to_peer(
                state,
                &room_id,
                &participant_id,
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
            debug!(
                participant_id = %participant_id,
                room_id = %room_id,
                "Received empty SFU event"
            );
        }
    }
}

async fn send_to_peer(
    state: &State,
    room_id: &str,
    participant_id: &str,
    payload: ServerMessage,
) {
    if participant_id.is_empty() {
        warn!("unable to route SFU event: empty participant_id");
        return;
    }

    match state
        .peers
        .send_to_peer(room_id, participant_id, payload)
        .await
    {
        DeliveryStatus::Delivered => {}
        DeliveryStatus::Missing => {
            debug!(
                room_id = %room_id,
                participant_id = %participant_id,
                "peer not connected locally"
            );
        }
        DeliveryStatus::Stale => {
            state
                .detach_peer(room_id, participant_id, "stale_sender")
                .await;
            let stale = state
                .peers
                .broadcast_to_room(
                    room_id,
                    Some(participant_id),
                    ServerMessage::ParticipantLeft {
                        nickname: participant_id.to_owned(),
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
    state: &State,
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

