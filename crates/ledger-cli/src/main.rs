pub mod proto {
    tonic::include_proto!("ledger");
}

mod tui;

use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use proto::ledger_client::LedgerClient;

fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".ledger").join("ledger.sock")
}

#[derive(Parser)]
#[command(name = "ledger", about = "Local telemetry daemon for Claude Code")]
struct Cli {
    /// Path to the daemon Unix socket
    #[arg(long, global = true)]
    socket: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Log a new event to the daemon
    Log {
        /// Event source (e.g., "pds", "haro")
        #[arg(long)]
        source: String,
        /// Event type (e.g., "skill_invoked", "agent_spawned")
        #[arg(long, name = "type")]
        event_type: String,
        /// JSON payload
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Query events from the daemon
    Query {
        #[command(subcommand)]
        command: QueryCommand,
    },
    /// Subscribe to real-time events
    Subscribe {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,
        /// Filter by event type
        #[arg(long, name = "type")]
        event_type: Option<String>,
    },
    /// Process a Claude Code hook event from stdin
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// Open interactive TUI
    Tui,
    /// Install the ledgerd daemon as a launchd service
    Install,
    /// Show daemon health status
    Status,
}

#[derive(Subcommand)]
enum QueryCommand {
    /// Search events with filters
    Events {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,
        /// Filter by event type
        #[arg(long, name = "type")]
        event_type: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value = "50")]
        limit: i64,
        /// Events after this ISO 8601 timestamp
        #[arg(long)]
        since: Option<String>,
    },
    /// List known event sources
    Sources,
    /// List known event types
    Types,
    /// Count events, optionally filtered
    Count {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,
        /// Filter by event type
        #[arg(long, name = "type")]
        event_type: Option<String>,
    },
}

#[derive(Subcommand)]
enum HookCommand {
    /// PostToolUse (Skill|Agent) — reads tool_name, tool_input
    Skill,
    /// PostToolUse (Write|Edit) — reads tool_input.file_path
    File,
    /// WorktreeCreate — reads name
    Worktree,
    /// InstructionsLoaded — logs session init
    Init,
    /// PreToolUse/PostToolUse — logs scrub event
    Scrub,
}

async fn connect_or_exit(socket_path: &str) -> LedgerClient<Channel> {
    match connect(socket_path).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub(crate) async fn connect(socket_path: &str) -> Result<LedgerClient<Channel>> {
    let path = PathBuf::from(socket_path);
    if !path.exists() {
        eprintln!(
            "Error: Daemon is not running. Socket not found at {}",
            socket_path
        );
        eprintln!("Start it with: ledger install");
        process::exit(1);
    }

    let socket_path = socket_path.to_string();
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

    Ok(LedgerClient::new(channel))
}

fn format_timestamp(ts: &prost_types::Timestamp) -> String {
    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| format!("{}s", ts.seconds))
}

fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, mins, secs)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

fn format_payload(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| payload.to_string()))
        .unwrap_or_else(|_| payload.to_string())
}

async fn cmd_log(client: &mut LedgerClient<Channel>, source: String, event_type: String, payload: String) -> Result<()> {
    // Validate JSON payload
    if serde_json::from_str::<serde_json::Value>(&payload).is_err() {
        eprintln!("Warning: payload is not valid JSON");
    }

    let response = client
        .log(proto::LogRequest {
            source,
            event_type,
            payload,
        })
        .await
        .context("Failed to log event")?
        .into_inner();

    println!("Event logged:");
    println!("  ID: {}", response.id);
    if let Some(ts) = &response.timestamp {
        println!("  Timestamp: {}", format_timestamp(ts));
    }

    Ok(())
}

fn parse_since(since: Option<String>) -> Result<Option<prost_types::Timestamp>> {
    since
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .or_else(|_| chrono::DateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S"))
                .map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                })
                .context("Invalid timestamp format. Use ISO 8601 (e.g., 2026-01-01T00:00:00Z)")
        })
        .transpose()
}

async fn cmd_query(client: &mut LedgerClient<Channel>, command: QueryCommand) -> Result<()> {
    match command {
        QueryCommand::Events { source, event_type, limit, since } => {
            cmd_query_events(client, source, event_type, limit, since).await
        }
        QueryCommand::Sources => cmd_query_sources(client).await,
        QueryCommand::Types => cmd_query_types(client).await,
        QueryCommand::Count { source, event_type } => {
            cmd_query_count(client, source, event_type).await
        }
    }
}

async fn cmd_query_events(
    client: &mut LedgerClient<Channel>,
    source: Option<String>,
    event_type: Option<String>,
    limit: i64,
    since: Option<String>,
) -> Result<()> {
    let since_ts = parse_since(since)?;

    let response = client
        .query(proto::QueryRequest {
            source: source.unwrap_or_default(),
            event_type: event_type.unwrap_or_default(),
            limit,
            since: since_ts,
            until: None,
        })
        .await
        .context("Failed to query events")?
        .into_inner();

    let events = &response.events;
    if events.is_empty() {
        println!("No events found.");
        return Ok(());
    }

    println!("Found {} event(s):\n", events.len());
    for event in events {
        let ts = event
            .timestamp
            .as_ref()
            .map(format_timestamp)
            .unwrap_or_else(|| "unknown".to_string());
        println!("--- {} ---", event.id);
        println!("  Time:   {}", ts);
        println!("  Source: {}", event.source);
        println!("  Type:   {}", event.event_type);
        println!("  Data:   {}", format_payload(&event.payload));
        println!();
    }

    Ok(())
}

async fn cmd_query_sources(client: &mut LedgerClient<Channel>) -> Result<()> {
    let response = client
        .query(proto::QueryRequest {
            source: String::new(),
            event_type: String::new(),
            limit: 0,
            since: None,
            until: None,
        })
        .await
        .context("Failed to query events")?
        .into_inner();

    let mut sources: Vec<String> = response
        .events
        .iter()
        .map(|e| e.source.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    sources.sort();

    if sources.is_empty() {
        println!("No sources found.");
    } else {
        for source in sources {
            println!("{}", source);
        }
    }

    Ok(())
}

async fn cmd_query_types(client: &mut LedgerClient<Channel>) -> Result<()> {
    let response = client
        .query(proto::QueryRequest {
            source: String::new(),
            event_type: String::new(),
            limit: 0,
            since: None,
            until: None,
        })
        .await
        .context("Failed to query events")?
        .into_inner();

    let mut types: Vec<String> = response
        .events
        .iter()
        .map(|e| e.event_type.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    types.sort();

    if types.is_empty() {
        println!("No event types found.");
    } else {
        for t in types {
            println!("{}", t);
        }
    }

    Ok(())
}

async fn cmd_query_count(
    client: &mut LedgerClient<Channel>,
    source: Option<String>,
    event_type: Option<String>,
) -> Result<()> {
    let response = client
        .query(proto::QueryRequest {
            source: source.unwrap_or_default(),
            event_type: event_type.unwrap_or_default(),
            limit: 0,
            since: None,
            until: None,
        })
        .await
        .context("Failed to query events")?
        .into_inner();

    println!("{}", response.events.len());

    Ok(())
}

async fn cmd_subscribe(
    client: &mut LedgerClient<Channel>,
    source: Option<String>,
    event_type: Option<String>,
) -> Result<()> {
    println!("Subscribing to events (Ctrl-C to stop)...\n");

    let mut stream = client
        .subscribe(proto::SubscribeRequest {
            source: source.unwrap_or_default(),
            event_type: event_type.unwrap_or_default(),
        })
        .await
        .context("Failed to subscribe to events")?
        .into_inner();

    use tokio_stream::StreamExt;
    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => {
                let ts = event
                    .timestamp
                    .as_ref()
                    .map(format_timestamp)
                    .unwrap_or_else(|| "now".to_string());
                println!(
                    "[{}] {} | {} | {}",
                    ts, event.source, event.event_type, event.payload
                );
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn read_stdin_json() -> Result<serde_json::Value> {
    let mut buf = String::new();
    tokio::io::stdin().read_to_string(&mut buf).await.context("Failed to read stdin")?;
    serde_json::from_str(&buf).context("Invalid JSON on stdin")
}

fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

async fn cmd_hook(client: &mut LedgerClient<Channel>, command: HookCommand) -> Result<()> {
    let session = std::env::var("CLAUDE_SESSION_ID").unwrap_or_else(|_| "unknown".into());

    match command {
        HookCommand::Skill => {
            let input = read_stdin_json().await?;
            let tool_name = json_str(&input, "tool_name").unwrap_or_default();
            let tool_input = input.get("tool_input");

            let (event_type, name) = match tool_name.as_str() {
                "Skill" => {
                    let skill = tool_input
                        .and_then(|ti| ti.get("skill"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    ("skill_invoked", skill.to_string())
                }
                "Agent" => {
                    let agent = tool_input
                        .and_then(|ti| ti.get("subagent_type").or_else(|| ti.get("description")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    ("agent_spawned", agent.to_string())
                }
                _ => return Ok(()),
            };

            let payload = serde_json::json!({
                "name": name,
                "session": session,
            });
            cmd_log(client, "pds".into(), event_type.into(), payload.to_string()).await?;
        }
        HookCommand::File => {
            let input = read_stdin_json().await?;
            let tool_input = input.get("tool_input");
            let file_path = tool_input
                .and_then(|ti| ti.get("file_path").or_else(|| ti.get("path")))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if file_path.is_empty() {
                return Ok(());
            }

            let ext = file_path
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase();

            let payload = serde_json::json!({
                "path": file_path,
                "ext": ext,
                "session": session,
            });
            cmd_log(client, "pds".into(), "file_modified".into(), payload.to_string()).await?;
        }
        HookCommand::Worktree => {
            let input = read_stdin_json().await?;
            let name = json_str(&input, "name").unwrap_or_else(|| "unknown".into());

            let payload = serde_json::json!({
                "name": name,
                "session": session,
            });
            cmd_log(client, "pds".into(), "worktree_created".into(), payload.to_string()).await?;
        }
        HookCommand::Init => {
            // Consume stdin (may be empty or {})
            let _ = read_stdin_json().await;

            let payload = serde_json::json!({
                "session": session,
            });
            cmd_log(client, "pds".into(), "instructions_loaded".into(), payload.to_string()).await?;
        }
        HookCommand::Scrub => {
            let input = read_stdin_json().await?;
            let tool_name = json_str(&input, "tool_name").unwrap_or_else(|| "unknown".into());

            let payload = serde_json::json!({
                "tool": tool_name,
                "session": session,
            });
            cmd_log(client, "pds".into(), "secret_scrubbed".into(), payload.to_string()).await?;
        }
    }

    Ok(())
}

async fn cmd_install() -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let ledger_dir = PathBuf::from(&home).join(".ledger");
    let bin_dir = ledger_dir.join("bin");

    // Create directories
    std::fs::create_dir_all(&bin_dir)
        .context("Failed to create ~/.ledger/bin/")?;

    println!("Building ledgerd in release mode...");
    let status = tokio::process::Command::new("cargo")
        .args(["build", "--release", "-p", "ledgerd"])
        .status()
        .await
        .context("Failed to run cargo build")?;

    if !status.success() {
        anyhow::bail!("cargo build failed");
    }

    // Find the release binary
    let release_bin = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()))
        .join("../../target/release/ledgerd");

    // Try to find the binary in common locations
    let binary_src = if release_bin.exists() {
        release_bin
    } else {
        // Try relative to CWD
        let cwd_bin = PathBuf::from("target/release/ledgerd");
        if cwd_bin.exists() {
            cwd_bin
        } else {
            anyhow::bail!("Could not find ledgerd binary. Expected at target/release/ledgerd");
        }
    };

    let binary_dest = bin_dir.join("ledgerd");
    std::fs::copy(&binary_src, &binary_dest)
        .with_context(|| format!("Failed to copy binary to {:?}", binary_dest))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_dest, std::fs::Permissions::from_mode(0o755))?;
    }

    println!("Installed ledgerd to {:?}", binary_dest);

    // Generate launchd plist
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ledger.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>5</integer>
    <key>StandardOutPath</key>
    <string>{}/ledgerd.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{}/ledgerd.stderr.log</string>
    <key>WorkingDirectory</key>
    <string>{}</string>
</dict>
</plist>"#,
        binary_dest.display(),
        ledger_dir.display(),
        ledger_dir.display(),
        home,
    );

    let launch_agents = PathBuf::from(&home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&launch_agents)
        .context("Failed to create ~/Library/LaunchAgents/")?;

    let plist_path = launch_agents.join("com.ledger.daemon.plist");

    // Unload existing service if present
    if plist_path.exists() {
        let _ = tokio::process::Command::new("launchctl")
            .args(["unload", plist_path.to_str().unwrap()])
            .status()
            .await;
    }

    std::fs::write(&plist_path, plist_content)
        .with_context(|| format!("Failed to write plist to {:?}", plist_path))?;

    println!("Installed plist to {:?}", plist_path);

    // Load the service
    let load_status = tokio::process::Command::new("launchctl")
        .args(["load", plist_path.to_str().unwrap()])
        .status()
        .await
        .context("Failed to run launchctl load")?;

    if load_status.success() {
        println!("Service loaded successfully.");

        // Wait briefly and check if socket exists
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let socket_path = ledger_dir.join("ledger.sock");
        if socket_path.exists() {
            println!("Daemon is running (socket at {:?})", socket_path);
        } else {
            println!("Warning: socket not found yet. Check logs at {:?}", ledger_dir.join("ledgerd.stderr.log"));
        }
    } else {
        eprintln!("Failed to load service. Try manually: launchctl load {:?}", plist_path);
    }

    Ok(())
}

async fn cmd_status(client: &mut LedgerClient<Channel>) -> Result<()> {
    let response = client
        .health(proto::HealthRequest {})
        .await
        .context("Failed to get daemon status")?
        .into_inner();

    println!("Ledger Daemon Status");
    println!("  Status:      {}", response.status);
    println!("  Uptime:      {}", format_duration(response.uptime_seconds));
    println!("  Events:      {}", response.event_count);
    println!("  Version:     {}", response.version);

    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let socket_path = cli
        .socket
        .unwrap_or_else(|| default_socket_path().to_string_lossy().to_string());

    let result = match cli.command {
        None | Some(Commands::Tui) => tui::run(socket_path).await,
        Some(Commands::Log {
            source,
            event_type,
            payload,
        }) => {
            let mut client = connect_or_exit(&socket_path).await;
            cmd_log(&mut client, source, event_type, payload).await
        }
        Some(Commands::Query { command }) => {
            let mut client = connect_or_exit(&socket_path).await;
            cmd_query(&mut client, command).await
        }
        Some(Commands::Subscribe {
            source,
            event_type,
        }) => {
            let mut client = connect_or_exit(&socket_path).await;
            cmd_subscribe(&mut client, source, event_type).await
        }
        Some(Commands::Hook { command }) => {
            let mut client = connect_or_exit(&socket_path).await;
            cmd_hook(&mut client, command).await
        }
        Some(Commands::Install) => cmd_install().await,
        Some(Commands::Status) => {
            let mut client = connect_or_exit(&socket_path).await;
            cmd_status(&mut client).await
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {:#}", e);
        process::exit(1);
    }
}
