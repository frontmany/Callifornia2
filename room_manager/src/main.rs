use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tonic::transport::{Endpoint, Server};
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

pub mod room_manager_pb {
    tonic::include_proto!("room_manager");
}

pub mod sfu_pb {
    tonic::include_proto!("sfu");
}

use room_manager_pb::room_manager_service_server::{RoomManagerService, RoomManagerServiceServer};
use room_manager_pb::{
    CloseRoomRequest, CloseRoomResponse, GetRoomRequest, GetRoomResponse, GetStatusRequest,
    GetStatusResponse, RoomAssignmentStatus, SfuStatus,
};
use sfu_pb::sfu_service_client::SfuServiceClient;
use sfu_pb::PingRequest;

const SFU_INSTANCES_KEY: &str = "room_manager:sfu_instances";
const SFU_LOAD_KEY: &str = "room_manager:sfu_load";
const WAITING_REQUESTS_KEY: &str = "room_manager:waiting_requests";
const ROOM_PREFIX: &str = "signaling:room:";

#[derive(Debug, Clone)]
struct Config {
    grpc_addr: String,
    redis_url: String,
    redis_connect_timeout: Duration,
    sfu_connect_timeout: Duration,
    health_interval: Duration,
    scale_down_idle_timeout: Duration,
    max_waiting_requests: usize,
    sfu_candidates: Vec<SfuCandidate>,
}

impl Config {
    fn from_env() -> Result<Self> {
        Ok(Self {
            grpc_addr: env_or("ROOM_MANAGER_ADDR", "0.0.0.0:50061"),
            redis_url: env_or("REDIS_URL", "redis://redis:6379/"),
            redis_connect_timeout: ms_env_or("REDIS_CONNECT_TIMEOUT_MS", 2_000)?,
            sfu_connect_timeout: ms_env_or("SFU_CONNECT_TIMEOUT_MS", 2_000)?,
            health_interval: ms_env_or("ROOM_MANAGER_HEALTH_INTERVAL_MS", 5_000)?,
            scale_down_idle_timeout: ms_env_or("ROOM_MANAGER_SCALE_DOWN_IDLE_MS", 300_000)?,
            max_waiting_requests: usize_env_or("ROOM_MANAGER_MAX_WAITING_REQUESTS", 10_000)?,
            sfu_candidates: parse_candidates(&env_or("ROOM_MANAGER_SFU_CANDIDATES", ""))?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SfuCandidate {
    instance_id: String,
    grpc_addr: String,
    max_clients: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SfuInstanceRecord {
    instance_id: String,
    grpc_addr: String,
    max_clients: u32,
    alive: bool,
    provisioned: bool,
    state: String,
    last_ping_unix: i64,
    idle_since_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WaitingRequestRecord {
    room_id: String,
    signaling_owner_id: String,
    signaling_owner_host: String,
    signaling_owner_port: u16,
    enqueued_at_unix: i64,
}

#[derive(Debug, Clone)]
struct RoomAssignment {
    room_id: String,
    owner_id: String,
    owner_host: String,
    owner_port: u16,
    state: String,
    sfu_instance_id: Option<String>,
    sfu_grpc_addr: Option<String>,
}

#[derive(Clone)]
struct StaticInventoryProvider {
    available: Arc<Mutex<VecDeque<SfuCandidate>>>,
    provisioned: Arc<Mutex<HashSet<String>>>,
}

impl StaticInventoryProvider {
    fn new(candidates: Vec<SfuCandidate>) -> Self {
        Self {
            available: Arc::new(Mutex::new(VecDeque::from(candidates))),
            provisioned: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    async fn provision_next(&self) -> Option<SfuCandidate> {
        let mut queue = self.available.lock().await;
        let candidate = queue.pop_front()?;
        self.provisioned
            .lock()
            .await
            .insert(candidate.instance_id.clone());
        Some(candidate)
    }

    async fn release(&self, instance_id: &str, candidate: SfuCandidate) {
        let mut provisioned = self.provisioned.lock().await;
        if !provisioned.remove(instance_id) {
            return;
        }
        drop(provisioned);

        self.available.lock().await.push_back(candidate);
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    redis: redis::Client,
    provider: StaticInventoryProvider,
}

#[derive(Clone)]
struct RoomManagerServer {
    state: AppState,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    init_tracing();

    let config = Arc::new(Config::from_env().context("load room manager config")?);
    let socket_addr: SocketAddr = config
        .grpc_addr
        .parse()
        .with_context(|| format!("invalid ROOM_MANAGER_ADDR: {}", config.grpc_addr))?;
    let redis = init_redis(&config.redis_url, config.redis_connect_timeout).await?;
    let provider = StaticInventoryProvider::new(config.sfu_candidates.clone());

    let state = AppState {
        config,
        redis,
        provider,
    };

    let health_state = state.clone();
    tokio::spawn(async move {
        health_loop(health_state).await;
    });

    info!(addr = %socket_addr, "Room Manager listening");
    Server::builder()
        .add_service(RoomManagerServiceServer::new(RoomManagerServer { state }))
        .serve(socket_addr)
        .await
        .context("room manager server failed")
}

#[tonic::async_trait]
impl RoomManagerService for RoomManagerServer {
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
                status: RoomAssignmentStatus::NotFound as i32,
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

        if let Some(candidate) = self.state.provider.provision_next().await {
            register_provisioning_instance(&self.state.redis, &candidate)
                .await
                .map_err(internal_status)?;
            info!(instance_id = %candidate.instance_id, grpc_addr = %candidate.grpc_addr, "Provisioned SFU candidate");
        } else {
            warn!("no static inventory SFU candidates left to provision");
        }

        Ok(Response::new(GetRoomResponse {
            status: RoomAssignmentStatus::Pending as i32,
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

async fn health_loop(state: AppState) {
    let interval = state.config.health_interval;
    loop {
        if let Err(err) = run_health_iteration(&state).await {
            error!(error = %err, "room manager health iteration failed");
        }
        tokio::time::sleep(interval).await;
    }
}

async fn run_health_iteration(state: &AppState) -> Result<()> {
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

async fn assign_waiting_requests(state: &AppState) -> Result<()> {
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

async fn scale_down_idle_instances(state: &AppState) -> Result<()> {
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

async fn ping_sfu(grpc_addr: &str, timeout: Duration) -> bool {
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

fn build_room_response(assignment: RoomAssignment) -> GetRoomResponse {
    let status = match assignment.state.as_str() {
        "active" => RoomAssignmentStatus::Assigned,
        "allocating" => RoomAssignmentStatus::Pending,
        _ => RoomAssignmentStatus::NotFound,
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

async fn init_redis(redis_url: &str, _connect_timeout: Duration) -> Result<redis::Client> {
    let client = redis::Client::open(redis_url).context("create redis client")?;
    let mut conn = client.get_connection_manager().await?;
    redis::cmd("PING")
        .query_async::<String>(&mut conn)
        .await
        .context("redis ping failed")?;
    Ok(client)
}

async fn select_least_loaded_instance(redis: &redis::Client) -> Result<Option<SfuInstanceRecord>> {
    let instances = list_sfu_instances(redis).await?;
    let loads = load_sfu_loads(redis).await?;
    let now = unix_now();

    let selected = instances
        .into_iter()
        .filter(|instance| {
            instance.alive
                && now - instance.last_ping_unix <= 30
                && instance.state == "ready"
                && loads
                    .get(&instance.instance_id)
                    .copied()
                    .unwrap_or_default()
                    < f64::from(instance.max_clients)
        })
        .min_by(|left, right| {
            let left_load = loads.get(&left.instance_id).copied().unwrap_or_default();
            let right_load = loads.get(&right.instance_id).copied().unwrap_or_default();
            left_load
                .partial_cmp(&right_load)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    Ok(selected)
}

async fn assign_room_to_instance(
    redis: &redis::Client,
    room_id: &str,
    signaling_owner_id: &str,
    signaling_owner_host: &str,
    signaling_owner_port: u16,
    instance: &SfuInstanceRecord,
) -> Result<RoomAssignment> {
    let mut conn = redis.get_connection_manager().await?;
    let meta_key = room_meta_key(room_id);

    let _: () = redis::pipe()
        .atomic()
        .hset(&meta_key, "owner_host", signaling_owner_host)
        .hset(&meta_key, "owner_port", signaling_owner_port)
        .hset(&meta_key, "signaling_owner_id", signaling_owner_id)
        .hset(&meta_key, "room_state", "active")
        .hset(&meta_key, "sfu_instance_id", &instance.instance_id)
        .hset(&meta_key, "sfu_grpc_addr", &instance.grpc_addr)
        .ignore()
        .query_async(&mut conn)
        .await?;

    increment_sfu_load(redis, &instance.instance_id).await?;

    Ok(RoomAssignment {
        room_id: room_id.to_owned(),
        owner_id: signaling_owner_id.to_owned(),
        owner_host: signaling_owner_host.to_owned(),
        owner_port: signaling_owner_port,
        state: "active".to_owned(),
        sfu_instance_id: Some(instance.instance_id.clone()),
        sfu_grpc_addr: Some(instance.grpc_addr.clone()),
    })
}

async fn get_room_assignment(
    redis: &redis::Client,
    room_id: &str,
) -> Result<Option<RoomAssignment>> {
    let mut conn = redis.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(room_meta_key(room_id)).await?;
    if values.is_empty() {
        return Ok(None);
    }

    let owner_host = values.get("owner_host").cloned().unwrap_or_default();
    let owner_port = values
        .get("owner_port")
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();

    Ok(Some(RoomAssignment {
        room_id: room_id.to_owned(),
        owner_id: values
            .get("signaling_owner_id")
            .cloned()
            .unwrap_or_default(),
        owner_host,
        owner_port,
        state: values
            .get("room_state")
            .cloned()
            .unwrap_or_else(|| "allocating".to_owned()),
        sfu_instance_id: values
            .get("sfu_instance_id")
            .cloned()
            .filter(|value| !value.is_empty()),
        sfu_grpc_addr: values
            .get("sfu_grpc_addr")
            .cloned()
            .filter(|value| !value.is_empty()),
    }))
}

async fn register_provisioning_instance(
    redis: &redis::Client,
    candidate: &SfuCandidate,
) -> Result<()> {
    let instance = SfuInstanceRecord {
        instance_id: candidate.instance_id.clone(),
        grpc_addr: candidate.grpc_addr.clone(),
        max_clients: candidate.max_clients,
        alive: false,
        provisioned: true,
        state: "starting".to_owned(),
        last_ping_unix: 0,
        idle_since_unix: unix_now(),
    };
    let payload = serde_json::to_string(&instance)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn
        .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
        .await?;
    let _: usize = conn.zadd(SFU_LOAD_KEY, &instance.instance_id, 0).await?;
    Ok(())
}

async fn delete_sfu_instance(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let _: () = redis::pipe()
        .atomic()
        .hdel(SFU_INSTANCES_KEY, instance_id)
        .ignore()
        .zrem(SFU_LOAD_KEY, instance_id)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

async fn list_sfu_instances(redis: &redis::Client) -> Result<Vec<SfuInstanceRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let values: HashMap<String, String> = conn.hgetall(SFU_INSTANCES_KEY).await?;
    values
        .into_values()
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .collect()
}

async fn load_sfu_loads(redis: &redis::Client) -> Result<HashMap<String, f64>> {
    let mut conn = redis.get_connection_manager().await?;
    let entries: Vec<(String, f64)> = conn.zrange_withscores(SFU_LOAD_KEY, 0, -1).await?;
    Ok(entries.into_iter().collect())
}

async fn increment_sfu_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let _: f64 = conn.zincr(SFU_LOAD_KEY, instance_id, 1).await?;
    update_idle_since(redis, instance_id, 0).await
}

async fn decrement_sfu_load(redis: &redis::Client, instance_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let current: f64 = conn.zscore(SFU_LOAD_KEY, instance_id).await.unwrap_or(0.0);
    let next = if current <= 0.0 { 0.0 } else { current - 1.0 };
    let _: usize = conn.zadd(SFU_LOAD_KEY, instance_id, next).await?;
    if next == 0.0 {
        update_idle_since(redis, instance_id, unix_now()).await?;
    }
    Ok(())
}

async fn update_idle_since(
    redis: &redis::Client,
    instance_id: &str,
    idle_since_unix: i64,
) -> Result<()> {
    let mut instance = get_sfu_instance(redis, instance_id)
        .await?
        .context("missing sfu instance for idle update")?;
    instance.idle_since_unix = idle_since_unix;
    save_sfu_instance(redis, &instance).await
}

async fn update_sfu_health(
    redis: &redis::Client,
    instance_id: &str,
    alive: bool,
    now: i64,
) -> Result<()> {
    let Some(mut instance) = get_sfu_instance(redis, instance_id).await? else {
        return Ok(());
    };
    instance.alive = alive;
    if alive {
        instance.last_ping_unix = now;
        instance.state = "ready".to_owned();
    }
    save_sfu_instance(redis, &instance).await
}

async fn get_sfu_instance(
    redis: &redis::Client,
    instance_id: &str,
) -> Result<Option<SfuInstanceRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let payload: Option<String> = conn.hget(SFU_INSTANCES_KEY, instance_id).await?;
    payload
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .transpose()
}

async fn save_sfu_instance(redis: &redis::Client, instance: &SfuInstanceRecord) -> Result<()> {
    let payload = serde_json::to_string(instance)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn
        .hset(SFU_INSTANCES_KEY, &instance.instance_id, payload)
        .await?;
    Ok(())
}

async fn enqueue_waiting_request(
    redis: &redis::Client,
    max_waiting_requests: usize,
    record: WaitingRequestRecord,
) -> Result<()> {
    let current = waiting_request_count(redis).await?;
    if current >= max_waiting_requests {
        return Ok(());
    }

    let payload = serde_json::to_string(&record)?;
    let mut conn = redis.get_connection_manager().await?;
    let _: usize = conn.rpush(WAITING_REQUESTS_KEY, payload).await?;

    let meta_key = room_meta_key(&record.room_id);
    let _: usize = conn
        .hset(&meta_key, "owner_host", &record.signaling_owner_host)
        .await?;
    let _: usize = conn
        .hset(&meta_key, "owner_port", record.signaling_owner_port)
        .await?;
    let _: usize = conn
        .hset(&meta_key, "signaling_owner_id", &record.signaling_owner_id)
        .await?;
    let _: usize = conn.hset(&meta_key, "room_state", "allocating").await?;
    Ok(())
}

async fn pop_waiting_request(redis: &redis::Client) -> Result<Option<WaitingRequestRecord>> {
    let mut conn = redis.get_connection_manager().await?;
    let payload: Option<String> = conn.lpop(WAITING_REQUESTS_KEY, None).await?;
    payload
        .map(|json| serde_json::from_str(&json).map_err(anyhow::Error::from))
        .transpose()
}

async fn waiting_request_count(redis: &redis::Client) -> Result<usize> {
    let mut conn = redis.get_connection_manager().await?;
    let len: usize = conn.llen(WAITING_REQUESTS_KEY).await?;
    Ok(len)
}

async fn count_active_rooms(redis: &redis::Client) -> Result<usize> {
    let mut cursor = 0;
    let mut total = 0;
    let mut conn = redis.get_connection_manager().await?;

    loop {
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("signaling:room:*:meta")
            .query_async(&mut conn)
            .await?;
        total += keys.len();
        if next == 0 {
            break;
        }
        cursor = next;
    }
    Ok(total)
}

async fn delete_room_assignment(redis: &redis::Client, room_id: &str) -> Result<()> {
    let mut conn = redis.get_connection_manager().await?;
    let _: () = redis::pipe()
        .atomic()
        .del(room_meta_key(room_id))
        .ignore()
        .del(room_members_key(room_id))
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

fn room_meta_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:meta")
}

fn room_members_key(room_id: &str) -> String {
    format!("{ROOM_PREFIX}{room_id}:members")
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn ms_env_or(key: &str, default_ms: u64) -> Result<Duration> {
    Ok(Duration::from_millis(u64_env_or(key, default_ms)?))
}

fn usize_env_or(key: &str, default: usize) -> Result<usize> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid integer for {key}: {value}")),
        Err(_) => Ok(default),
    }
}

fn u64_env_or(key: &str, default: u64) -> Result<u64> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid integer for {key}: {value}")),
        Err(_) => Ok(default),
    }
}

fn parse_candidates(value: &str) -> Result<Vec<SfuCandidate>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    value
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let mut parts = entry.split('|');
            let instance_id = parts
                .next()
                .filter(|value| !value.is_empty())
                .context("candidate instance_id is required")?;
            let grpc_addr = parts
                .next()
                .filter(|value| !value.is_empty())
                .context("candidate grpc_addr is required")?;
            let max_clients = parts
                .next()
                .context("candidate max_clients is required")?
                .parse()
                .context("candidate max_clients must be integer")?;

            Ok(SfuCandidate {
                instance_id: instance_id.to_owned(),
                grpc_addr: grpc_addr.to_owned(),
                max_clients,
            })
        })
        .collect()
}

fn internal_status(err: anyhow::Error) -> Status {
    Status::internal(err.to_string())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
