use std::sync::Arc;
use std::time::Instant;

use crate::proto;
use crate::store::EventStore;

/// Tracks daemon health and produces HealthResponse.
pub struct HealthService {
    start_time: Instant,
    store: Arc<EventStore>,
}

impl HealthService {
    pub fn new(store: Arc<EventStore>) -> Self {
        Self {
            start_time: Instant::now(),
            store,
        }
    }

    pub fn health_response(&self) -> proto::HealthResponse {
        let uptime = self.start_time.elapsed().as_secs();
        let event_count = self.store.event_count().unwrap_or(0);

        proto::HealthResponse {
            status: "healthy".to_string(),
            uptime_seconds: uptime,
            event_count,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}
