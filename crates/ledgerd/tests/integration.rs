//! Integration tests: start ledgerd, exercise gRPC API via ledger-client, tear down.

use std::path::PathBuf;
use std::time::Duration;

use ledger_client::{LedgerClient, QueryFilters, SubscribeFilters};
use tokio::time::timeout;

/// Start ledgerd with temp socket + db paths, return (child, socket_path).
fn start_daemon(dir: &std::path::Path) -> (std::process::Child, PathBuf) {
    let socket = dir.join("ledger.sock");
    let db = dir.join("ledger.db");

    // CARGO_BIN_EXE_ledgerd is set by Cargo for integration tests in the same package
    let bin = env!("CARGO_BIN_EXE_ledgerd");
    let child = std::process::Command::new(bin)
        .args([
            "--socket",
            socket.to_str().unwrap(),
            "--db",
            db.to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("Failed to start ledgerd at {:?}: {}", bin, e));

    (child, socket)
}

/// Check if we can bind a Unix socket in the given directory.
/// Returns false inside sandboxed environments that block socket creation.
fn can_bind_unix_socket(dir: &std::path::Path) -> bool {
    let test_sock = dir.join(".probe.sock");
    match std::os::unix::net::UnixListener::bind(&test_sock) {
        Ok(_) => {
            let _ = std::fs::remove_file(&test_sock);
            true
        }
        Err(_) => false,
    }
}

/// Poll until the Unix socket appears (daemon ready).
async fn wait_for_socket(socket: &std::path::Path) {
    for _ in 0..50 {
        if socket.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("Daemon did not create socket at {:?} within 5s", socket);
}

#[tokio::test]
async fn test_log_and_query_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    if !can_bind_unix_socket(dir.path()) {
        eprintln!("Skipping: sandbox does not allow Unix socket binding");
        return;
    }
    let (mut child, socket) = start_daemon(dir.path());
    wait_for_socket(&socket).await;

    let mut client = LedgerClient::connect_to_path(&socket).await.unwrap();

    // Log an event
    let resp = client
        .log("integration-test", "ping", r#"{"round":"trip"}"#)
        .await
        .unwrap();
    assert!(!resp.id.is_empty());

    // Query it back
    let events = client
        .query(QueryFilters {
            event_type: Some("ping".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, resp.id);
    assert_eq!(events[0].source, "integration-test");
    assert_eq!(events[0].payload, r#"{"round":"trip"}"#);

    child.kill().ok();
    child.wait().ok();
}

#[tokio::test]
async fn test_health_shows_event_count() {
    let dir = tempfile::tempdir().unwrap();
    if !can_bind_unix_socket(dir.path()) {
        eprintln!("Skipping: sandbox does not allow Unix socket binding");
        return;
    }
    let (mut child, socket) = start_daemon(dir.path());
    wait_for_socket(&socket).await;

    let mut client = LedgerClient::connect_to_path(&socket).await.unwrap();

    client.log("test", "a", "{}").await.unwrap();
    client.log("test", "b", "{}").await.unwrap();

    let health = client.health().await.unwrap();
    assert_eq!(health.status, "healthy");
    assert_eq!(health.event_count, 2);
    assert!(!health.version.is_empty());

    child.kill().ok();
    child.wait().ok();
}

#[tokio::test]
async fn test_subscribe_receives_live_events() {
    let dir = tempfile::tempdir().unwrap();
    if !can_bind_unix_socket(dir.path()) {
        eprintln!("Skipping: sandbox does not allow Unix socket binding");
        return;
    }
    let (mut child, socket) = start_daemon(dir.path());
    wait_for_socket(&socket).await;

    let mut subscriber = LedgerClient::connect_to_path(&socket).await.unwrap();
    let mut writer = LedgerClient::connect_to_path(&socket).await.unwrap();

    // Open subscription
    let mut stream = subscriber
        .subscribe(SubscribeFilters::default())
        .await
        .unwrap();

    // Let it establish
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Write from a separate client
    writer
        .log("sub-test", "streamed", r#"{"live":true}"#)
        .await
        .unwrap();

    // Receive on the subscription
    let event = timeout(Duration::from_secs(5), stream.message())
        .await
        .expect("Timed out waiting for streamed event")
        .expect("Stream error")
        .expect("Stream ended");

    assert_eq!(event.source, "sub-test");
    assert_eq!(event.event_type, "streamed");

    child.kill().ok();
    child.wait().ok();
}

#[tokio::test]
async fn test_query_filters_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    if !can_bind_unix_socket(dir.path()) {
        eprintln!("Skipping: sandbox does not allow Unix socket binding");
        return;
    }
    let (mut child, socket) = start_daemon(dir.path());
    wait_for_socket(&socket).await;

    let mut client = LedgerClient::connect_to_path(&socket).await.unwrap();

    client.log("pds", "skill_invoked", "{}").await.unwrap();
    client.log("pds", "agent_spawned", "{}").await.unwrap();
    client.log("haro", "skill_invoked", "{}").await.unwrap();

    // By source
    let events = client
        .query(QueryFilters {
            source: Some("pds".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 2);

    // By type
    let events = client
        .query(QueryFilters {
            event_type: Some("skill_invoked".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 2);

    // By source + type
    let events = client
        .query(QueryFilters {
            source: Some("pds".to_string()),
            event_type: Some("skill_invoked".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 1);

    // Limit
    let events = client
        .query(QueryFilters {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 1);

    child.kill().ok();
    child.wait().ok();
}
