pub mod proto {
    tonic::include_proto!("ledger");
}

mod health;
mod migrations;
mod server;
mod store;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;
use tracing::{info, warn};

use health::HealthService;
use proto::ledger_server::LedgerServer;
use server::LedgerService;
use store::EventStore;

fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".ledger").join("ledger.sock")
}

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".ledger").join("ledger.db")
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Parse CLI args (simple --socket and --db flags for testing)
    let args: Vec<String> = std::env::args().collect();
    let socket_path = parse_flag(&args, "--socket").unwrap_or_else(default_socket_path);
    let db_path = parse_flag(&args, "--db").unwrap_or_else(default_db_path);

    // Create parent directory if needed
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;
    }

    // Remove stale socket file
    if socket_path.exists() {
        warn!("Removing stale socket file at {:?}", socket_path);
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("Failed to remove stale socket {:?}", socket_path))?;
    }

    // Open SQLite store
    info!("Opening database at {:?}", db_path);
    let store = Arc::new(EventStore::new(&db_path)?);
    let health = Arc::new(HealthService::new(Arc::clone(&store)));

    // Create gRPC service
    let service = LedgerService::new(Arc::clone(&store), Arc::clone(&health));

    // Bind Unix socket
    info!("Listening on {:?}", socket_path);
    let uds = UnixListener::bind(&socket_path)
        .with_context(|| format!("Failed to bind Unix socket at {:?}", socket_path))?;
    let uds_stream = UnixListenerStream::new(uds);

    // Start gRPC server with graceful shutdown
    let shutdown = async {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Received SIGINT, shutting down...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
            }
        }
    };

    info!("ledgerd v{} started", env!("CARGO_PKG_VERSION"));

    Server::builder()
        .add_service(LedgerServer::new(service))
        .serve_with_incoming_shutdown(uds_stream, shutdown)
        .await?;

    // Clean up socket on graceful shutdown
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    info!("ledgerd shut down cleanly");
    Ok(())
}

fn parse_flag(args: &[String], flag: &str) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}
