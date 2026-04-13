use anyhow::Result;
use ledger_client::{Event, HealthResponse, LedgerClient, QueryFilters, SubscribeFilters};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

/// Messages from the daemon background task to the main loop.
#[derive(Debug)]
pub enum DaemonMsg {
    /// Initial batch of historical events (newest first).
    History(Vec<Event>),
    /// A single live event from the subscription stream.
    LiveEvent(Event),
    /// Health poll result.
    Health(HealthResponse),
    /// Connection or stream error.
    Error(String),
}

/// Spawns a background task that:
/// 1. Connects to the daemon
/// 2. Fetches recent history (Query RPC)
/// 3. Polls health periodically
/// 4. Streams live events (Subscribe RPC)
pub fn spawn_daemon_bridge(socket_path: Option<String>) -> mpsc::UnboundedReceiver<DaemonMsg> {
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(e) = run_bridge(socket_path, tx.clone()).await {
            let _ = tx.send(DaemonMsg::Error(e.to_string()));
        }
    });

    rx
}

async fn run_bridge(
    socket_path: Option<String>,
    tx: mpsc::UnboundedSender<DaemonMsg>,
) -> Result<()> {
    let mut client = LedgerClient::connect(socket_path.as_deref()).await?;

    // 1. Fetch initial history
    let events = client
        .query(QueryFilters {
            limit: Some(500),
            ..Default::default()
        })
        .await?;
    let _ = tx.send(DaemonMsg::History(events));

    // 2. Fetch initial health
    if let Ok(health) = client.health().await {
        let _ = tx.send(DaemonMsg::Health(health));
    }

    // 3. Spawn health poller
    let health_tx = tx.clone();
    let health_socket = socket_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            match LedgerClient::connect(health_socket.as_deref()).await {
                Ok(mut c) => {
                    if let Ok(health) = c.health().await {
                        if health_tx.send(DaemonMsg::Health(health)).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => {
                    // Health poll failed — non-fatal, will retry
                }
            }
        }
    });

    // 4. Stream live events
    let mut stream = client
        .subscribe(SubscribeFilters::default())
        .await?;

    while let Some(event) = stream.next().await {
        match event {
            Ok(e) => {
                if tx.send(DaemonMsg::LiveEvent(e)).is_err() {
                    break;
                }
            }
            Err(e) => {
                let _ = tx.send(DaemonMsg::Error(format!("Stream error: {}", e)));
                break;
            }
        }
    }

    Ok(())
}
