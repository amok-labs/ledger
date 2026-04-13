use anyhow::Result;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::proto;

/// Messages from the daemon background task to the main loop.
#[derive(Debug)]
pub enum DaemonMsg {
    /// Initial batch of historical events (newest first).
    History(Vec<proto::Event>),
    /// A single live event from the subscription stream.
    LiveEvent(proto::Event),
    /// Health poll result.
    Health(proto::HealthResponse),
    /// Connection or stream error.
    Error(String),
}

/// Spawns a background task that:
/// 1. Connects to the daemon
/// 2. Fetches recent history (Query RPC)
/// 3. Polls health periodically
/// 4. Streams live events (Subscribe RPC)
pub fn spawn_daemon_bridge(socket_path: String) -> mpsc::UnboundedReceiver<DaemonMsg> {
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(e) = run_bridge(socket_path, tx.clone()).await {
            let _ = tx.send(DaemonMsg::Error(e.to_string()));
        }
    });

    rx
}

async fn run_bridge(
    socket_path: String,
    tx: mpsc::UnboundedSender<DaemonMsg>,
) -> Result<()> {
    let mut client = crate::connect(&socket_path).await?;

    // 1. Fetch initial history
    let response = client
        .query(proto::QueryRequest {
            source: String::new(),
            event_type: String::new(),
            limit: 500,
            since: None,
            until: None,
        })
        .await?
        .into_inner();
    let _ = tx.send(DaemonMsg::History(response.events));

    // 2. Fetch initial health
    if let Ok(health) = client.health(proto::HealthRequest {}).await {
        let _ = tx.send(DaemonMsg::Health(health.into_inner()));
    }

    // 3. Spawn health poller
    let health_tx = tx.clone();
    let health_socket = socket_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            match crate::connect(&health_socket).await {
                Ok(mut c) => {
                    if let Ok(health) = c.health(proto::HealthRequest {}).await {
                        if health_tx.send(DaemonMsg::Health(health.into_inner())).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => {}
            }
        }
    });

    // 4. Stream live events
    let mut stream = client
        .subscribe(proto::SubscribeRequest {
            source: String::new(),
            event_type: String::new(),
        })
        .await?
        .into_inner();

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
