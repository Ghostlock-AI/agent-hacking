use anyhow::Result;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::io::{self, Write};
use termion::raw::IntoRawMode;
use tokio::io::AsyncReadExt;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

#[derive(Parser, Debug)]
#[command(author, version, about = "RAT client - Connect to remote shell")]
struct Args {
    /// Server URL (e.g., https://example.ngrok-free.dev)
    url: String,

    /// Session ID to reconnect to (optional)
    #[arg(short, long)]
    session: Option<String>,

    /// Stop a session
    #[arg(short = 'k', long)]
    stop: Option<String>,
}

#[derive(Deserialize)]
struct SessionCreateResponse {
    session_id: String,
    ws_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle stop session
    if let Some(session_id) = args.stop {
        stop_session(&args.url, &session_id).await?;
        println!("Session {} stopped", session_id);
        return Ok(());
    }

    // Get or create session
    let ws_url = if let Some(session_id) = args.session {
        // Reconnect to existing session
        let base = args.url.replace("https://", "wss://").replace("http://", "ws://");
        format!("{}/shell/{}", base, session_id)
    } else {
        // Create new session
        let response = create_session(&args.url).await?;
        println!("🔗 Created session: {}", response.session_id);
        println!("🔗 Connecting to remote shell...\n");
        response.ws_url
    };

    // Connect WebSocket
    let (ws_stream, _) = connect_async(&ws_url).await?;
    println!("[REMOTE] Connected!\n");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Put terminal in raw mode
    let mut stdout = io::stdout().into_raw_mode()?;

    // Channel for shutdown coordination
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let shutdown_tx2 = shutdown_tx.clone();

    // Task 1: Read from stdin, send to WebSocket
    let stdin_task = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 1024];

        loop {
            tokio::select! {
                result = stdin.read(&mut buf) => {
                    match result {
                        Ok(n) if n > 0 => {
                            let data = buf[..n].to_vec();
                            if ws_tx.send(Message::Binary(data)).await.is_err() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    });

    // Task 2: Read from WebSocket, write to stdout
    let stdout_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(data) => {
                    if stdout.write_all(&data).is_err() {
                        break;
                    }
                    if stdout.flush().is_err() {
                        break;
                    }
                }
                Message::Text(text) => {
                    if stdout.write_all(text.as_bytes()).is_err() {
                        break;
                    }
                    if stdout.flush().is_err() {
                        break;
                    }
                }
                Message::Close(_) => {
                    let _ = shutdown_tx2.send(()).await;
                    break;
                }
                _ => {}
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = stdin_task => {},
        _ = stdout_task => {},
    }

    println!("\n🔌 Disconnected");

    Ok(())
}

async fn create_session(base_url: &str) -> Result<SessionCreateResponse> {
    let client = reqwest::Client::new();
    let url = format!("{}/session/create", base_url);

    let response = client.post(&url)
        .send()
        .await?
        .json::<SessionCreateResponse>()
        .await?;

    Ok(response)
}

async fn stop_session(base_url: &str, session_id: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/session/{}/stop", base_url, session_id);

    client.post(&url).send().await?;

    Ok(())
}
