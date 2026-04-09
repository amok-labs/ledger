use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

use crate::health::HealthService;
use crate::proto;
use crate::proto::ledger_server::Ledger;
use crate::store::EventStore;

/// gRPC service implementation for the Ledger daemon.
pub struct LedgerService {
    store: Arc<EventStore>,
    health: Arc<HealthService>,
    event_tx: broadcast::Sender<proto::Event>,
}

impl LedgerService {
    pub fn new(store: Arc<EventStore>, health: Arc<HealthService>) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            store,
            health,
            event_tx,
        }
    }
}

#[tonic::async_trait]
impl Ledger for LedgerService {
    async fn log(
        &self,
        request: Request<proto::LogRequest>,
    ) -> Result<Response<proto::LogResponse>, Status> {
        let req = request.into_inner();

        let event = self
            .store
            .insert_event(&req.source, &req.event_type, &req.payload)
            .map_err(|e| Status::internal(format!("Failed to insert event: {}", e)))?;

        // Broadcast to subscribers (ignore error if no receivers)
        let _ = self.event_tx.send(event.clone());

        Ok(Response::new(proto::LogResponse {
            id: event.id,
            timestamp: event.timestamp,
        }))
    }

    async fn query(
        &self,
        request: Request<proto::QueryRequest>,
    ) -> Result<Response<proto::QueryResponse>, Status> {
        let req = request.into_inner();

        let events = self
            .store
            .query_events(
                &req.source,
                &req.event_type,
                req.limit,
                req.since.as_ref(),
                req.until.as_ref(),
            )
            .map_err(|e| Status::internal(format!("Failed to query events: {}", e)))?;

        Ok(Response::new(proto::QueryResponse { events }))
    }

    type SubscribeStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<proto::Event, Status>> + Send>>;

    async fn subscribe(
        &self,
        request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let req = request.into_inner();
        let rx = self.event_tx.subscribe();

        let source_filter = if req.source.is_empty() {
            None
        } else {
            Some(req.source)
        };
        let type_filter = if req.event_type.is_empty() {
            None
        } else {
            Some(req.event_type)
        };

        let stream = BroadcastStream::new(rx)
            .filter_map(move |result| {
                match result {
                    Ok(event) => {
                        // Apply filters
                        if let Some(ref src) = source_filter {
                            if &event.source != src {
                                return None;
                            }
                        }
                        if let Some(ref et) = type_filter {
                            if &event.event_type != et {
                                return None;
                            }
                        }
                        Some(Ok(event))
                    }
                    Err(_) => None, // Skip lagged messages
                }
            });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn health(
        &self,
        _request: Request<proto::HealthRequest>,
    ) -> Result<Response<proto::HealthResponse>, Status> {
        Ok(Response::new(self.health.health_response()))
    }
}
