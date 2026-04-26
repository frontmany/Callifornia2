use anyhow::Result;
use tonic::transport::Endpoint;
use tracing::{error, info};

use crate::models::{SfuCandidate, State};
use crate::proto::sfu_pb::sfu_service_client::SfuServiceClient;
use crate::proto::sfu_pb::PingRequest;
use crate::storage::{
    assign_room_to_instance, delete_sfu_instance, list_sfu_instances, load_sfu_loads,
    pop_waiting_request, select_least_loaded_instance, update_sfu_health,
};
use crate::util::unix_now;

pub async fn health_loop(state: State) {
    let interval = state.config.health_interval;
    loop {
        if let Err(err) = run_health_iteration(&state).await {
            error!(error = %err, "room manager health iteration failed");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_health_iteration(state: &State) -> Result<()> {
    let instances = list_sfu_instances(&state.redis).await?;
    let now = unix_now();

    for instance in instances {
        let alive = ping_sfu(&instance.grpc_addr, state.config.sfu_connect_timeout).await;
        update_sfu_health(&state.redis, &instance.instance_id, alive, now).await?;
    }

    assign_waiting_requests(state).await?;
    scale_down_idle_instances(state).await?;
    Ok(())
}

async fn assign_waiting_requests(state: &State) -> Result<()> {
    loop {
        let Some(instance) = select_least_loaded_instance(&state.redis).await? else {
            break;
        };
        let Some(request) = pop_waiting_request(&state.redis).await? else {
            break;
        };

        assign_room_to_instance(
            &state.redis,
            &request.room_id,
            &request.signaling_owner_id,
            &request.signaling_owner_host,
            request.signaling_owner_port,
            &instance,
        )
        .await?;
        info!(room_id = %request.room_id, instance_id = %instance.instance_id, "Assigned waiting room to SFU");
    }

    Ok(())
}

async fn scale_down_idle_instances(state: &State) -> Result<()> {
    let instances = list_sfu_instances(&state.redis).await?;
    let loads = load_sfu_loads(&state.redis).await?;
    let now = unix_now();

    for instance in instances {
        let load = loads
            .get(&instance.instance_id)
            .copied()
            .unwrap_or_default();
        if load > 0.0 || !instance.provisioned {
            continue;
        }
        if instance.idle_since_unix <= 0 {
            continue;
        }
        if now - instance.idle_since_unix < state.config.scale_down_idle_timeout.as_secs() as i64 {
            continue;
        }

        delete_sfu_instance(&state.redis, &instance.instance_id).await?;
        let instance_id = instance.instance_id.clone();
        state
            .provider
            .release(
                &instance_id,
                SfuCandidate {
                    instance_id: instance_id.clone(),
                    grpc_addr: instance.grpc_addr,
                    max_clients: instance.max_clients,
                },
            )
            .await;
    }

    Ok(())
}

async fn ping_sfu(grpc_addr: &str, timeout: std::time::Duration) -> bool {
    // TODO(health): добавить backoff/счетчик ошибок и переход в "failed",
    // чтобы не держать вечный "starting" при постоянных проблемах старта.
    let endpoint = match Endpoint::from_shared(grpc_addr.to_owned()) {
        Ok(endpoint) => endpoint.timeout(timeout),
        Err(_) => return false,
    };
    let channel = match endpoint.connect().await {
        Ok(channel) => channel,
        Err(_) => return false,
    };
    SfuServiceClient::new(channel)
        .ping(PingRequest {})
        .await
        .is_ok()
}
