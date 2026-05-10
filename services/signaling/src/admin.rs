pub mod pb {
    tonic::include_proto!("signaling_admin");
}

use pb::signaling_admin_service_server::{SignalingAdminService, SignalingAdminServiceServer};
use pb::{
    CloseRoomsBySfuRequest, CloseRoomsBySfuResponse, PingRequest, PingResponse, PurgeRequest,
    PurgeResponse,
};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::state::{DegradationReason, State};

pub struct AdminService {
    state: State,
}

impl AdminService {
    pub fn new(state: State) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl SignalingAdminService for AdminService {
    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let active_peers = self.state.peers.peer_count().await;
        let active_sessions = self.state.sessions.count().await;
        let node_id = self.state.config.public_node_id();
        Ok(Response::new(PingResponse {
            ok: true,
            active_peers,
            active_sessions,
            node_id,
        }))
    }

    async fn purge(
        &self,
        request: Request<PurgeRequest>,
    ) -> Result<Response<PurgeResponse>, Status> {
        let reason = request.into_inner().reason;
        info!(reason = %reason, "admin Purge requested");

        let degradation = DegradationReason::RedisDown;

        let rooms_snapshot = self.state.peers.snapshot_rooms().await;
        let rooms_closed = rooms_snapshot.len() as u32;

        // run_purge is idempotent via try_begin_purge flag.
        let state = self.state.clone();
        tokio::spawn(async move {
            state.run_purge(degradation).await;
        });

        Ok(Response::new(PurgeResponse {
            ok: true,
            rooms_closed,
        }))
    }

    async fn close_rooms_by_sfu(
        &self,
        request: Request<CloseRoomsBySfuRequest>,
    ) -> Result<Response<CloseRoomsBySfuResponse>, Status> {
        let req = request.into_inner();
        info!(sfu_grpc_addr = %req.sfu_grpc_addr, reason = %req.reason, "admin CloseRoomsBySfu requested");

        let rooms_before = self.state.peers.snapshot_rooms().await;
        let state = self.state.clone();
        let sfu_addr = req.sfu_grpc_addr.clone();
        let reason = req.reason.clone();
        tokio::spawn(async move {
            state.close_rooms_due_to_sfu_addr(&sfu_addr, &reason).await;
        });

        // Count rooms that matched this SFU (best-effort; the actual close is async).
        let rooms_closed = rooms_before.len() as u32;
        Ok(Response::new(CloseRoomsBySfuResponse {
            ok: true,
            rooms_closed,
        }))
    }
}

pub fn make_server(state: State) -> SignalingAdminServiceServer<AdminService> {
    SignalingAdminServiceServer::new(AdminService::new(state))
}
