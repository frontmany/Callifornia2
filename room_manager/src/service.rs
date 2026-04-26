use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::models::{RoomBinding, RoomManager, WaitingRequestRecord};
use crate::proto::room_manager_pb::room_manager_service_server::RoomManagerService;
use crate::proto::room_manager_pb::{
    CloseRoomRequest, CloseRoomResponse, GetRoomRequest, GetRoomResponse, GetStatusRequest,
    GetStatusResponse, RoomBindingStatus, SfuStatus,
};
use crate::storage::{
    assign_room_to_instance, count_active_rooms, decrement_sfu_load, delete_room_assignment,
    enqueue_waiting_request, get_room_assignment, has_provisioning_instance, list_sfu_instances,
    load_sfu_loads, register_provisioning_instance, select_least_loaded_instance,
    waiting_request_count,
};
use crate::util::{internal_status, unix_now};

#[tonic::async_trait]
impl RoomManagerService for RoomManager {
    async fn get_room(
        &self,
        request: Request<GetRoomRequest>,
    ) -> Result<Response<GetRoomResponse>, Status> {
        let request = request.into_inner();
        let owner_port = u16::try_from(request.signaling_owner_port)
            .map_err(|_| Status::invalid_argument("signaling_owner_port out of range"))?;

        if let Some(assignment) = get_room_assignment(&self.state.redis, &request.room_id)
            .await
            .map_err(internal_status)?
        {
            return Ok(Response::new(build_room_response(assignment)));
        }

        if !request.create_if_missing {
            return Ok(Response::new(GetRoomResponse {
                status: RoomBindingStatus::NotFound as i32,
                room_id: request.room_id,
                message: "room not found".to_owned(),
                ..Default::default()
            }));
        }

        if let Some(instance) = select_least_loaded_instance(&self.state.redis)
            .await
            .map_err(internal_status)?
        {
            let assignment = assign_room_to_instance(
                &self.state.redis,
                &request.room_id,
                &request.signaling_owner_id,
                &request.signaling_owner_host,
                owner_port,
                &instance,
            )
            .await
            .map_err(internal_status)?;
            return Ok(Response::new(build_room_response(assignment)));
        }

        enqueue_waiting_request(
            &self.state.redis,
            self.state.config.max_waiting_requests,
            WaitingRequestRecord {
                room_id: request.room_id.clone(),
                signaling_owner_id: request.signaling_owner_id.clone(),
                signaling_owner_host: request.signaling_owner_host.clone(),
                signaling_owner_port: owner_port,
                enqueued_at_unix: unix_now(),
            },
        )
        .await
        .map_err(internal_status)?;

        if has_provisioning_instance(&self.state.redis)
            .await
            .map_err(internal_status)?
        {
            info!("SFU provisioning already in progress; waiting for capacity");
        } else if let Some(candidate) = self.state.provider.provision_next().await {
            // TODO(provisioner): здесь сейчас только логическая "резервация" кандидата.
            // Нужно вызывать внешний оркестратор (Docker/K8s/Cloud API),
            // запускать реальный SFU инстанс и только после успешного старта
            // регистрировать его в Redis.
            register_provisioning_instance(&self.state.redis, &candidate)
                .await
                .map_err(internal_status)?;
            info!(instance_id = %candidate.instance_id, grpc_addr = %candidate.grpc_addr, "Provisioned SFU candidate");
        } else {
            warn!("no static inventory SFU candidates left to provision");
        }

        Ok(Response::new(GetRoomResponse {
            status: RoomBindingStatus::Pending as i32,
            room_id: request.room_id,
            signaling_owner_id: request.signaling_owner_id,
            signaling_owner_host: request.signaling_owner_host,
            signaling_owner_port: u32::from(owner_port),
            room_state: "allocating".to_owned(),
            message: "no SFU capacity available yet; retry later".to_owned(),
            ..Default::default()
        }))
    }

    async fn close_room(
        &self,
        request: Request<CloseRoomRequest>,
    ) -> Result<Response<CloseRoomResponse>, Status> {
        let request = request.into_inner();
        let Some(assignment) = get_room_assignment(&self.state.redis, &request.room_id)
            .await
            .map_err(internal_status)?
        else {
            return Ok(Response::new(CloseRoomResponse {
                closed: false,
                message: "room not found".to_owned(),
            }));
        };

        if let Some(instance_id) = assignment.sfu_instance_id.as_deref() {
            decrement_sfu_load(&self.state.redis, instance_id)
                .await
                .map_err(internal_status)?;
        }
        delete_room_assignment(&self.state.redis, &request.room_id)
            .await
            .map_err(internal_status)?;

        Ok(Response::new(CloseRoomResponse {
            closed: true,
            message: "room closed".to_owned(),
        }))
    }

    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        let instances = list_sfu_instances(&self.state.redis)
            .await
            .map_err(internal_status)?;
        let loads = load_sfu_loads(&self.state.redis)
            .await
            .map_err(internal_status)?;
        let active_rooms = count_active_rooms(&self.state.redis)
            .await
            .map_err(internal_status)?;
        let pending_requests = waiting_request_count(&self.state.redis)
            .await
            .map_err(internal_status)?;

        let sfu_instances = instances
            .into_iter()
            .map(|instance| SfuStatus {
                instance_id: instance.instance_id.clone(),
                grpc_addr: instance.grpc_addr,
                max_clients: instance.max_clients,
                current_load: loads
                    .get(&instance.instance_id)
                    .copied()
                    .unwrap_or_default() as u32,
                alive: instance.alive,
                last_ping_unix: instance.last_ping_unix,
                provisioned: instance.provisioned,
            })
            .collect();

        Ok(Response::new(GetStatusResponse {
            sfu_instances,
            active_rooms: active_rooms as u32,
            pending_requests: pending_requests as u32,
        }))
    }
}

fn build_room_response(assignment: RoomBinding) -> GetRoomResponse {
    let status = match assignment.state.as_str() {
        "active" => RoomBindingStatus::Assigned,
        "allocating" => RoomBindingStatus::Pending,
        _ => RoomBindingStatus::NotFound,
    };

    GetRoomResponse {
        status: status as i32,
        room_id: assignment.room_id,
        sfu_instance_id: assignment.sfu_instance_id.unwrap_or_default(),
        sfu_grpc_addr: assignment.sfu_grpc_addr.unwrap_or_default(),
        signaling_owner_id: assignment.owner_id,
        signaling_owner_host: assignment.owner_host,
        signaling_owner_port: u32::from(assignment.owner_port),
        room_state: assignment.state,
        message: String::new(),
    }
}
