//! # ledger-client
//!
//! Shared Rust client library for connecting to the ledgerd daemon via gRPC over Unix socket.
//!
//! ## Example
//!
//! ```rust,no_run
//! use ledger_client::LedgerClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut client = LedgerClient::connect_default().await?;
//!     let response = client.log("test", "ping", "{}").await?;
//!     println!("Logged event: {}", response.id);
//!     Ok(())
//! }
//! ```

pub mod proto {
    tonic::include_proto!("ledger");
}

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

// Re-export key proto types for consumer convenience
pub use proto::Event;
pub use proto::HealthResponse;
pub use proto::LogResponse;

/// Filters for querying events.
#[derive(Debug, Default, Clone)]
pub struct QueryFilters {
    /// Filter by source (empty = all).
    pub source: Option<String>,
    /// Filter by event type (empty = all).
    pub event_type: Option<String>,
    /// Maximum number of results (None = no limit).
    pub limit: Option<i64>,
    /// Return events after this timestamp.
    pub since: Option<prost_types::Timestamp>,
    /// Return events before this timestamp.
    pub until: Option<prost_types::Timestamp>,
}

/// Filters for subscribing to events.
#[derive(Debug, Default, Clone)]
pub struct SubscribeFilters {
    /// Filter by source (empty = all).
    pub source: Option<String>,
    /// Filter by event type (empty = all).
    pub event_type: Option<String>,
}

/// Ergonomic client for the ledgerd daemon.
///
/// Wraps the raw tonic gRPC client with a user-friendly API
/// that handles Unix socket connection, error messages, and type conversion.
pub struct LedgerClient {
    inner: proto::ledger_client::LedgerClient<Channel>,
}

impl LedgerClient {
    /// Connect to ledgerd via Unix socket at the given path.
    ///
    /// If `socket_path` is `None`, uses the default path `~/.ledger/ledger.sock`.
    pub async fn connect(socket_path: Option<&str>) -> Result<Self> {
        let path = match socket_path {
            Some(p) => PathBuf::from(p),
            None => default_socket_path(),
        };
        Self::connect_to_path(&path).await
    }

    /// Connect to ledgerd using the default socket path (`~/.ledger/ledger.sock`).
    pub async fn connect_default() -> Result<Self> {
        let path = default_socket_path();
        Self::connect_to_path(&path).await
    }

    /// Connect to ledgerd via Unix socket at the specified path.
    pub async fn connect_to_path(socket_path: &Path) -> Result<Self> {
        if !socket_path.exists() {
            bail!(
                "Daemon not running: socket not found at {:?}. Start it with: ledger install",
                socket_path
            );
        }

        let socket_path = socket_path.to_path_buf();
        let channel = Endpoint::try_from("http://[::]:50051")
            .context("Failed to create endpoint")?
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = socket_path.clone();
                async move {
                    let stream = UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await
            .context("Failed to connect to ledgerd. Is the daemon running?")?;

        let inner = proto::ledger_client::LedgerClient::new(channel);
        Ok(Self { inner })
    }

    /// Log a new event to the daemon.
    ///
    /// Returns the generated event ID and timestamp.
    pub async fn log(
        &mut self,
        source: &str,
        event_type: &str,
        payload: &str,
    ) -> Result<proto::LogResponse> {
        let request = proto::LogRequest {
            source: source.to_string(),
            event_type: event_type.to_string(),
            payload: payload.to_string(),
        };

        let response = self
            .inner
            .log(request)
            .await
            .context("Failed to log event")?;

        Ok(response.into_inner())
    }

    /// Query events with optional filters.
    ///
    /// Returns a list of matching events.
    pub async fn query(&mut self, filters: QueryFilters) -> Result<Vec<proto::Event>> {
        let request = proto::QueryRequest {
            source: filters.source.unwrap_or_default(),
            event_type: filters.event_type.unwrap_or_default(),
            limit: filters.limit.unwrap_or(0),
            since: filters.since,
            until: filters.until,
        };

        let response = self
            .inner
            .query(request)
            .await
            .context("Failed to query events")?;

        Ok(response.into_inner().events)
    }

    /// Subscribe to real-time events from the daemon.
    ///
    /// Returns a gRPC streaming response that yields events as they arrive.
    pub async fn subscribe(
        &mut self,
        filters: SubscribeFilters,
    ) -> Result<tonic::Streaming<proto::Event>> {
        let request = proto::SubscribeRequest {
            source: filters.source.unwrap_or_default(),
            event_type: filters.event_type.unwrap_or_default(),
        };

        let response = self
            .inner
            .subscribe(request)
            .await
            .context("Failed to subscribe to events")?;

        Ok(response.into_inner())
    }

    /// Get the daemon's health status.
    ///
    /// Returns status, uptime, event count, and version.
    pub async fn health(&mut self) -> Result<proto::HealthResponse> {
        let response = self
            .inner
            .health(proto::HealthRequest {})
            .await
            .context("Failed to get health status")?;

        Ok(response.into_inner())
    }
}

/// Returns the default socket path: `~/.ledger/ledger.sock`.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".ledger").join("ledger.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        assert!(path.to_str().unwrap().ends_with(".ledger/ledger.sock"));
    }

    #[test]
    fn test_query_filters_default() {
        let filters = QueryFilters::default();
        assert!(filters.source.is_none());
        assert!(filters.event_type.is_none());
        assert!(filters.limit.is_none());
        assert!(filters.since.is_none());
        assert!(filters.until.is_none());
    }

    #[test]
    fn test_subscribe_filters_default() {
        let filters = SubscribeFilters::default();
        assert!(filters.source.is_none());
        assert!(filters.event_type.is_none());
    }

    #[tokio::test]
    async fn test_connect_missing_socket() {
        let result = LedgerClient::connect(Some("/tmp/nonexistent-ledger.sock")).await;
        assert!(result.is_err());
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected error"),
        };
        assert!(err.contains("Daemon not running"), "Got: {}", err);
    }
}
