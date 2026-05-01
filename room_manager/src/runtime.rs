use anyhow::Result;
use tonic::transport::Endpoint;
use tracing::{error, info, warn};

use crate::models::{SfuCandidate, State};
use crate::proto::sfu_pb::sfu_service_client::SfuServiceClient;
use crate::proto::sfu_pb::PingRequest;
use crate::storage::{
    alive_signaling_nodes, assign_room_to_instance, decrement_sfu_room_load,
    delete_signaling_room_members, delete_signaling_session, delete_sfu_instance,
    get_room_assignment, get_session_owner, list_sfu_instances, load_sfu_room_loads,
    pop_waiting_request, remove_signaling_load, remove_signaling_node_heartbeats_below,
    scan_room_binding_ids, scan_signaling_room_ids, scan_signaling_session_ids,
    select_least_loaded_instance, signaling_load_members, update_sfu_health,
};
use crate::util::unix_now;
use std::collections::HashSet;

pub async fn janitor_loop(state: State) {
    let interval = state.config.janitor_interval;
    loop {
        if let Err(err) = run_janitor_iteration(&state).await {
            error!(error = %err, "janitor iteration failed");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_janitor_iteration(state: &State) -> Result<()> {
    let stale_threshold = state.config.signaling_stale_timeout;
    let alive_nodes = alive_signaling_nodes(&state.redis, stale_threshold).await?;
    let alive_set: HashSet<String> = alive_nodes.into_iter().collect();
    let now = unix_now();
    let cutoff = now - stale_threshold.as_secs() as i64;

    let binding_ids = scan_room_binding_ids(&state.redis).await?;
    for room_id in &binding_ids {
        let Some(binding) = get_room_assignment(&state.redis, room_id).await? else {
            continue;
        };
        if binding.owner_id.is_empty() || alive_set.contains(&binding.owner_id) {
            continue;
        }
        warn!(
            room_id = %room_id,
            owner = %binding.owner_id,
            "janitor: reclaiming room from stale signaling node"
        );
        if let Some(sfu_instance_id) = binding.sfu_instance_id.as_deref() {
            if let Err(err) = decrement_sfu_room_load(&state.redis, sfu_instance_id).await {
                warn!(error = %err, room_id = %room_id, "janitor: failed to decrement sfu load");
            }
        }
        if let Err(err) =
            crate::storage::delete_room_assignment(&state.redis, room_id).await
        {
            warn!(error = %err, room_id = %room_id, "janitor: failed to delete room binding");
        }
        if let Err(err) = delete_signaling_room_members(&state.redis, room_id).await {
            warn!(error = %err, room_id = %room_id, "janitor: failed to delete room members");
        }
    }

    let signaling_room_ids = scan_signaling_room_ids(&state.redis).await?;
    let binding_set: HashSet<String> = binding_ids.into_iter().collect();
    for room_id in &signaling_room_ids {
        if !binding_set.contains(room_id) {
            warn!(room_id = %room_id, "janitor: deleting orphan signaling room members");
            if let Err(err) = delete_signaling_room_members(&state.redis, room_id).await {
                warn!(error = %err, room_id = %room_id, "janitor: failed to delete orphan room members");
            }
        }
    }

    let load_members = signaling_load_members(&state.redis).await?;
    for node_id in load_members {
        if !alive_set.contains(&node_id) {
            warn!(node = %node_id, "janitor: removing stale signaling instance load");
            if let Err(err) = remove_signaling_load(&state.redis, &node_id).await {
                warn!(error = %err, node = %node_id, "janitor: failed to remove stale load");
            }
        }
    }

    let session_ids = scan_signaling_session_ids(&state.redis).await?;
    for session_id in session_ids {
        let owner = match get_session_owner(&state.redis, &session_id).await? {
            Some(owner) => owner,
            None => continue,
        };
        if alive_set.contains(&owner) {
            continue;
        }
        warn!(
            session_id = %session_id,
            owner = %owner,
            "janitor: deleting session of stale signaling node"
        );
        if let Err(err) = delete_signaling_session(&state.redis, &session_id).await {
            warn!(error = %err, session_id = %session_id, "janitor: failed to delete stale session");
        }
    }

    if let Err(err) = remove_signaling_node_heartbeats_below(&state.redis, cutoff).await {
        warn!(error = %err, "janitor: failed to clean stale signaling node heartbeats");
    }

    Ok(())
}

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
    let provisioning_timeout = state.config.sfu_provisioning_timeout.as_secs() as i64;

    for instance in instances {
        let alive = ping_sfu(&instance.grpc_addr, state.config.sfu_connect_timeout).await;
        if let Err(err) =
            update_sfu_health(&state.redis, &instance.instance_id, alive, now).await
        {
            warn!(error = %err, instance_id = %instance.instance_id, "failed to update sfu health");
            continue;
        }

        if !alive
            && matches!(instance.state.as_str(), "starting" | "provisioning")
            && instance.idle_since_unix > 0
            && now - instance.idle_since_unix > provisioning_timeout
        {
            warn!(
                instance_id = %instance.instance_id,
                "sfu provisioning timed out; reclaiming slot"
            );
            if let Err(err) =
                delete_sfu_instance(&state.redis, &instance.instance_id).await
            {
                warn!(error = %err, instance_id = %instance.instance_id, "failed to delete stale provisioning instance");
                continue;
            }
            state
                .provider
                .release(
                    &instance.instance_id,
                    SfuCandidate {
                        instance_id: instance.instance_id.clone(),
                        grpc_addr: instance.grpc_addr,
                        max_rooms: instance.max_rooms,
                    },
                )
                .await;
        }
    }

    if let Err(err) = assign_waiting_requests(state).await {
        warn!(error = %err, "assign_waiting_requests failed in health iteration");
    }
    if let Err(err) = scale_down_idle_instances(state).await {
        warn!(error = %err, "scale_down_idle_instances failed in health iteration");
    }
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
    let room_loads = load_sfu_room_loads(&state.redis).await?;
    let now = unix_now();

    for instance in instances {
        let room_load = room_loads
            .get(&instance.instance_id)
            .copied()
            .unwrap_or_default();
        if room_load > 0.0 || !instance.provisioned {
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
                    max_rooms: instance.max_rooms,
                },
            )
            .await;
    }

    Ok(())
}

async fn ping_sfu(grpc_addr: &str, connect_timeout: std::time::Duration) -> bool {
    let endpoint = match Endpoint::from_shared(grpc_addr.to_owned()) {
        Ok(endpoint) => endpoint
            .connect_timeout(connect_timeout)
            .timeout(connect_timeout),
        Err(_) => return false,
    };
    let channel = match endpoint.connect().await {
        Ok(channel) => channel,
        Err(_) => return false,
    };
    matches!(
        tokio::time::timeout(
            connect_timeout,
            SfuServiceClient::new(channel).ping(PingRequest {})
        )
        .await,
        Ok(Ok(_))
    )
}
