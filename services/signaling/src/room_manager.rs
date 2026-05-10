pub mod pb {
    tonic::include_proto!("room_manager");
}

use pb::room_manager_service_client::RoomManagerServiceClient;
use pb::{CloseRoomRequest, GetRoomRequest, RoomBindingStatus};
use tonic::transport::Channel;

#[derive(Debug, Clone)]
pub struct RoomBinding {
    pub sfu_grpc_addr: Option<String>,
    pub room_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStatus {
    Assigned,
    Pending,
    NotFound,
}

#[derive(Debug, thiserror::Error)]
pub enum RoomManagerError {
    #[error("room manager request failed")]
    Transport(#[from] tonic::Status),
    #[error("invalid room assignment returned by room manager")]
    InvalidAssignment,
}

#[derive(Clone)]
pub struct Client {
    channel: Channel,
}

impl Client {
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }

    pub async fn get_room(
        &self,
        room_id: &str,
        signaling_owner_id: &str,
        signaling_owner_host: &str,
        signaling_owner_port: u16,
        create_if_missing: bool,
    ) -> Result<(BindingStatus, RoomBinding), RoomManagerError> {
        let response = self
            .grpc_client()
            .get_room(GetRoomRequest {
                room_id: room_id.to_owned(),
                signaling_owner_id: signaling_owner_id.to_owned(),
                signaling_owner_host: signaling_owner_host.to_owned(),
                signaling_owner_port: u32::from(signaling_owner_port),
                create_if_missing,
            })
            .await?
            .into_inner();

        let status = match RoomBindingStatus::try_from(response.status)
            .unwrap_or(RoomBindingStatus::Unspecified)
        {
            RoomBindingStatus::Assigned => BindingStatus::Assigned,
            RoomBindingStatus::Pending => BindingStatus::Pending,
            RoomBindingStatus::NotFound | RoomBindingStatus::Unspecified => BindingStatus::NotFound,
        };

        let _ = u16::try_from(response.signaling_owner_port)
            .map_err(|_| RoomManagerError::InvalidAssignment)?;
        let binding = RoomBinding {
            sfu_grpc_addr: non_empty(response.sfu_grpc_addr),
            room_state: response.room_state,
        };

        Ok((status, binding))
    }

    pub async fn close_room(&self, room_id: &str) -> Result<bool, RoomManagerError> {
        let response = self
            .grpc_client()
            .close_room(CloseRoomRequest {
                room_id: room_id.to_owned(),
            })
            .await?
            .into_inner();
        Ok(response.closed)
    }

    fn grpc_client(&self) -> RoomManagerServiceClient<Channel> {
        RoomManagerServiceClient::new(self.channel.clone())
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
