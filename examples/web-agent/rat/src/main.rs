use axum::{
    extract::Json,
    http::StatusCode,
    response::{IntoResponse, Response, sse::Event},
    routing::{get, post},
    Router,
};
use clap::Parser;
use daemonize::Daemonize;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tower_http::cors::CorsLayer;
use tracing::{info, error};
use tracing_subscriber;

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
}

/// Health check endpoint
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
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

    let app = create_router();

    let addr = format!("{}:{}", args.host, args.port);
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Server listening on {}", addr);
    info!("Endpoints:");
    info!("  GET  /health          - Health check");
    info!("  POST /execute         - Execute command and return full output");
    info!("  POST /execute/stream  - Execute command and stream output");

    axum::serve(listener, app).await?;

    Ok(())
}
