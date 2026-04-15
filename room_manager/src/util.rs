use std::time::{SystemTime, UNIX_EPOCH};

use tonic::Status;
use tracing_subscriber::EnvFilter;

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

pub fn internal_status(err: anyhow::Error) -> Status {
    Status::internal(err.to_string())
}

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
