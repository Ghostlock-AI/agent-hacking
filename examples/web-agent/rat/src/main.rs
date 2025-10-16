use axum::{
    extract::{Json, Path, WebSocketUpgrade, ws::{WebSocket, Message}},
    http::StatusCode,
    response::{IntoResponse, Response, sse::Event},
    routing::{get, post},
    Router,
};
use clap::Parser;
use daemonize::Daemonize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{info, error, warn};
use tracing_subscriber;
use uuid::Uuid;
use portable_pty::{PtySize, CommandBuilder, native_pty_system, PtyPair};
use futures::{StreamExt, SinkExt};

lazy_static::lazy_static! {
    static ref PUBLIC_URL: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    static ref SESSIONS: Arc<Mutex<HashMap<String, Arc<Mutex<PtySession>>>>> = Arc::new(Mutex::new(HashMap::new()));
}

struct PtySession {
    id: String,
    pty_pair: PtyPair,
    master_taken: bool,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Run as daemon (background process)
    #[arg(short, long)]
    daemon: bool,

    /// Port to bind to
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Enable ngrok tunnel for internet access
    #[arg(short, long)]
    ngrok: bool,
}

#[derive(Deserialize, Serialize)]
struct CommandRequest {
    command: String,
    args: Option<Vec<String>>,
    working_dir: Option<String>,
}

#[derive(Serialize)]
struct CommandResponse {
    success: bool,
    output: String,
    error: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    public_url: Option<String>,
}

#[derive(Serialize)]
struct SessionCreateResponse {
    session_id: String,
    ws_url: String,
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    active: bool,
}

#[derive(Deserialize)]
struct NgrokTunnel {
    public_url: String,
}

#[derive(Deserialize)]
struct NgrokApiResponse {
    tunnels: Vec<NgrokTunnel>,
}

/// Health check endpoint
async fn health() -> Json<HealthResponse> {
    let public_url = PUBLIC_URL.lock().unwrap().clone();
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_url,
    })
}

/// Create a new PTY session
async fn create_session() -> Result<Json<SessionCreateResponse>, (StatusCode, String)> {
    info!("Creating new PTY session");

    let session_id = Uuid::new_v4().to_string();

    // Create PTY
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            error!("Failed to create PTY: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create PTY: {}", e))
        })?;

    // Spawn shell in PTY
    let mut cmd = CommandBuilder::new("bash");
    cmd.env("TERM", "xterm-256color");

    let _child = pty_pair.slave.spawn_command(cmd).map_err(|e| {
        error!("Failed to spawn shell: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to spawn shell: {}", e))
    })?;

    let session = PtySession {
        id: session_id.clone(),
        pty_pair,
        master_taken: false,
    };

    // Store session
    SESSIONS.lock().unwrap().insert(session_id.clone(), Arc::new(Mutex::new(session)));

    let public_url = PUBLIC_URL.lock().unwrap().clone();
    let ws_url = if let Some(url) = public_url {
        format!("{}/shell/{}", url.replace("http", "ws"), session_id)
    } else {
        format!("ws://localhost:3000/shell/{}", session_id)
    };

    info!("Created session {} with WebSocket URL: {}", session_id, ws_url);

    Ok(Json(SessionCreateResponse {
        session_id,
        ws_url,
    }))
}

/// List all sessions
async fn list_sessions() -> Json<Vec<SessionInfo>> {
    let sessions = SESSIONS.lock().unwrap();
    let list: Vec<SessionInfo> = sessions
        .keys()
        .map(|id| SessionInfo {
            id: id.clone(),
            active: true,
        })
        .collect();
    Json(list)
}

/// Stop a session
async fn stop_session(Path(session_id): Path<String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    info!("Stopping session {}", session_id);

    let mut sessions = SESSIONS.lock().unwrap();
    if sessions.remove(&session_id).is_some() {
        Ok(Json(serde_json::json!({"status": "stopped"})))
    } else {
        Err((StatusCode::NOT_FOUND, "Session not found".to_string()))
    }
}

/// WebSocket handler for shell I/O
async fn shell_ws_handler(ws: WebSocketUpgrade, Path(session_id): Path<String>) -> Response {
    info!("WebSocket connection request for session {}", session_id);

    ws.on_upgrade(move |socket| handle_shell_socket(socket, session_id))
}

async fn handle_shell_socket(socket: WebSocket, session_id: String) {
    let session_id_log = session_id.clone();
    info!("WebSocket connected for session {}", session_id_log);

    // Get session
    let session = {
        let sessions = SESSIONS.lock().unwrap();
        sessions.get(&session_id).cloned()
    };

    let session = match session {
        Some(s) => s,
        None => {
            error!("Session {} not found", session_id);
            return;
        }
    };

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Get PTY master (can only be taken once per session) - must drop lock immediately
    let (mut pty_reader, mut pty_master) = {
        let mut session_lock = session.lock().unwrap();
        if session_lock.master_taken {
            error!("Session {} master already taken", session_id);
            return;
        }
        session_lock.master_taken = true;

        // Clone reader before taking writer
        let reader = session_lock.pty_pair.master.try_clone_reader().unwrap();
        let writer = session_lock.pty_pair.master.take_writer().unwrap();
        (reader, writer)
    }; // lock dropped here

    // Channels for PTY I/O
    let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(100);
    let (ws_to_pty_tx, mut ws_to_pty_rx) = mpsc::channel::<Vec<u8>>(100);

    // Task 1: PTY reader (blocking I/O in separate thread)
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let data = buf[..n].to_vec();
                    if pty_tx.blocking_send(data).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    // Task 2: PTY writer (blocking I/O in separate thread)
    std::thread::spawn(move || {
        use std::io::Write;
        while let Some(data) = ws_to_pty_rx.blocking_recv() {
            if pty_master.write_all(&data).is_err() {
                break;
            }
            if pty_master.flush().is_err() {
                break;
            }
        }
    });

    // Task 3: PTY → WebSocket
    let session_id_clone = session_id.clone();
    let read_task = tokio::spawn(async move {
        while let Some(data) = pty_rx.recv().await {
            if ws_tx.send(Message::Binary(data)).await.is_err() {
                break;
            }
        }
        info!("PTY→WS task ended for session {}", session_id_clone);
    });

    // Task 4: WebSocket → PTY
    let session_id_clone2 = session_id.clone();
    let write_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(data) => {
                    if ws_to_pty_tx.send(data).await.is_err() {
                        break;
                    }
                }
                Message::Text(text) => {
                    if ws_to_pty_tx.send(text.into_bytes()).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        info!("WS→PTY task ended for session {}", session_id_clone2);
    });

    // Wait for both tasks to complete
    let _ = tokio::join!(read_task, write_task);
    info!("WebSocket disconnected for session {}", session_id_log);
}

/// Start ngrok tunnel and return public URL
async fn start_ngrok(port: u16) -> anyhow::Result<String> {
    info!("Starting ngrok tunnel on port {}", port);

    // Configure ngrok with auth token from env
    if let Ok(token) = std::env::var("NGROK_AUTHTOKEN") {
        Command::new("ngrok")
            .args(&["config", "add-authtoken", &token])
            .output()
            .await?;
    }

    // Spawn ngrok process
    let mut child = Command::new("ngrok")
        .args(&["http", &port.to_string(), "--log", "stdout"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // Give ngrok time to start
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Query ngrok API to get public URL
    let client = reqwest::Client::new();
    for _ in 0..10 {
        match client.get("http://127.0.0.1:4040/api/tunnels").send().await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<NgrokApiResponse>().await {
                    if let Some(tunnel) = data.tunnels.first() {
                        let url = tunnel.public_url.clone();
                        info!("🌍 PUBLIC URL: {}", url);
                        info!("🌍 Access your server from anywhere at: {}", url);

                        // Store the URL globally
                        *PUBLIC_URL.lock().unwrap() = Some(url.clone());

                        return Ok(url);
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }

    // Keep ngrok process alive in background
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    Err(anyhow::anyhow!("Failed to get ngrok URL"))
}

/// Execute a command and return the output
async fn execute_command(
    Json(payload): Json<CommandRequest>,
) -> Result<Json<CommandResponse>, (StatusCode, String)> {
    info!("Executing command: {} with args: {:?}", payload.command, payload.args);

    let mut cmd = Command::new(&payload.command);

    if let Some(args) = &payload.args {
        cmd.args(args);
    }

    if let Some(working_dir) = &payload.working_dir {
        cmd.current_dir(working_dir);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| {
            error!("Failed to execute command: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to execute command: {}", e))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let response = CommandResponse {
        success: output.status.success(),
        output: stdout,
        error: if stderr.is_empty() { None } else { Some(stderr) },
    };

    Ok(Json(response))
}

/// Execute a command and stream output line by line
async fn execute_command_stream(
    Json(payload): Json<CommandRequest>,
) -> Response {
    info!("Streaming command: {} with args: {:?}", payload.command, payload.args);

    let mut cmd = Command::new(&payload.command);

    if let Some(args) = &payload.args {
        cmd.args(args);
    }

    if let Some(working_dir) = &payload.working_dir {
        cmd.current_dir(working_dir);
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            error!("Failed to spawn command: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to spawn command: {}", e)
            ).into_response();
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let stream = async_stream::stream! {
        let mut stdout_lines = stdout_reader.lines();
        let mut stderr_lines = stderr_reader.lines();

        loop {
            tokio::select! {
                result = stdout_lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            yield Ok::<_, anyhow::Error>(Event::default().data(format!("stdout: {}", line)));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            yield Ok(Event::default().data(format!("error: {}", e)));
                            break;
                        }
                    }
                }
                result = stderr_lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            yield Ok::<_, anyhow::Error>(Event::default().data(format!("stderr: {}", line)));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            yield Ok(Event::default().data(format!("error: {}", e)));
                            break;
                        }
                    }
                }
                else => break,
            }
        }

        // Wait for the command to complete
        match child.wait().await {
            Ok(status) => {
                yield Ok(Event::default().data(format!("exit_code: {}", status.code().unwrap_or(-1))));
            }
            Err(e) => {
                yield Ok(Event::default().data(format!("error: {}", e)));
            }
        }
    };

    axum::response::sse::Sse::new(stream).into_response()
}

fn create_router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute_command))
        .route("/execute/stream", post(execute_command_stream))
        .route("/session/create", post(create_session))
        .route("/sessions", get(list_sessions))
        .route("/session/:session_id/stop", post(stop_session))
        .route("/shell/:session_id", get(shell_ws_handler))
        .layer(CorsLayer::permissive())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rat=info,tower_http=info".into()),
        )
        .init();

    // If daemon mode is requested, daemonize the process
    if args.daemon {
        info!("Starting in daemon mode...");
        let daemonize = Daemonize::new()
            .pid_file("/tmp/rat.pid")
            .working_directory("/tmp");

        match daemonize.start() {
            Ok(_) => info!("Daemonized successfully"),
            Err(e) => {
                error!("Error daemonizing: {}", e);
                return Err(anyhow::anyhow!("Failed to daemonize: {}", e));
            }
        }
    }

    // Start ngrok if requested
    if args.ngrok {
        match start_ngrok(args.port).await {
            Ok(_) => {},
            Err(e) => {
                warn!("Failed to start ngrok: {}. Continuing without public URL.", e);
            }
        }
    }

    let app = create_router();

    let addr = format!("{}:{}", args.host, args.port);
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server listening on {}", addr);
    info!("Endpoints:");
    info!("  GET  /health               - Health check");
    info!("  POST /execute              - Execute command and return full output");
    info!("  POST /execute/stream       - Execute command and stream output");
    info!("  POST /session/create       - Create new shell session");
    info!("  GET  /sessions             - List active sessions");
    info!("  POST /session/:id/stop     - Stop a session");
    info!("  WS   /shell/:id            - WebSocket shell connection");

    axum::serve(listener, app).await?;

    Ok(())
}
