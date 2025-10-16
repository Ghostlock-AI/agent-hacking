# WebSocket Shell Connection - How It Works

## Architecture Overview

```
[Client (Your Laptop)] <--WebSocket--> [Server (Docker/ngrok)] <--> [PTY Shell (bash)]
```

## How WebSocket Communication Works in Rust

### Server Side (`rat/src/main.rs`)

**1. Session Creation (HTTP POST `/session/create`)**
```rust
// Server spawns a PTY (pseudo-terminal) with bash
let pty_pair = pty_system.openpty(PtySize { rows: 24, cols: 80, ... })?;
let mut cmd = CommandBuilder::new("bash");
pty_pair.slave.spawn_command(cmd)?;

// Store session with UUID
SESSIONS.insert(session_id, Arc::new(Mutex::new(session)));

// Return WebSocket URL
{
    "session_id": "abc-123",
    "ws_url": "wss://your-ngrok-url.com/shell/abc-123"
}
```

**2. WebSocket Connection (WS `/shell/:session_id`)**
```rust
async fn handle_shell_socket(socket: WebSocket, session_id: String) {
    // Split socket into send/receive halves
    let (ws_tx, ws_rx) = socket.split();

    // Get PTY reader/writer
    let pty_reader = session.pty_pair.master.try_clone_reader()?;
    let pty_writer = session.pty_pair.master.try_clone_writer()?;

    // Task 1: PTY stdout → WebSocket (server sends output to client)
    tokio::spawn(async move {
        loop {
            let data = pty_reader.read(&mut buf)?;  // Read from bash
            ws_tx.send(Message::Binary(data))?;      // Send to client
        }
    });

    // Task 2: WebSocket → PTY stdin (client sends keystrokes to bash)
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            let data = msg.into_data();
            pty_writer.write_all(&data)?;  // Write to bash stdin
        }
    });
}
```

**Data Flow (Server):**
```
User types 'l' on client
    ↓
WebSocket receives binary frame [0x6C]
    ↓
Server writes [0x6C] to PTY stdin
    ↓
Bash processes 'l' keystroke
    ↓
Bash outputs to PTY stdout (nothing yet, waiting for Enter)
```

When user presses Enter:
```
WebSocket receives [0x0A] (newline)
    ↓
PTY stdin receives [0x0A]
    ↓
Bash executes command 'l'
    ↓
Bash writes "command not found" to PTY stdout
    ↓
Server reads from PTY stdout
    ↓
Server sends binary frames through WebSocket to client
```

### Client Side (`rat-client/src/main.rs`)

**1. Create Session**
```rust
let response = reqwest::post("/session/create").await?;
// Gets: { session_id: "...", ws_url: "wss://..." }
```

**2. Connect WebSocket**
```rust
let (ws_stream, _) = connect_async(&ws_url).await?;
let (ws_tx, ws_rx) = ws_stream.split();
```

**3. Put Terminal in Raw Mode**
```rust
let mut stdout = io::stdout().into_raw_mode()?;
// Raw mode: no line buffering, no echo, every keystroke sent immediately
```

**4. Bidirectional Communication**
```rust
// Task 1: Local stdin → WebSocket (send keystrokes to server)
tokio::spawn(async move {
    loop {
        let data = stdin.read(&mut buf)?;        // Read keystroke
        ws_tx.send(Message::Binary(data))?;      // Send to server
    }
});

// Task 2: WebSocket → Local stdout (receive output from server)
tokio::spawn(async move {
    while let Some(msg) = ws_rx.next().await {
        let data = msg.into_data();
        stdout.write_all(&data)?;   // Print to screen
        stdout.flush()?;
    }
});
```

**Data Flow (Client):**
```
User presses 'l' key
    ↓
Raw mode captures it immediately (no buffering)
    ↓
stdin.read() gets [0x6C]
    ↓
Send WebSocket binary frame [0x6C] to server
    ↓
(Server processes it...)
    ↓
WebSocket receives binary frames from server
    ↓
Write directly to stdout (screen shows bash output)
```

## Why This Works Like SSH

1. **Raw Terminal Mode**: Every keystroke sent immediately, no local echo
2. **Binary Frames**: Raw bytes, not JSON - just like network packets
3. **PTY (Pseudo-Terminal)**: Real bash shell with full terminal capabilities
4. **Bidirectional**: Simultaneous send/receive via async tasks

## Testing Instructions

### 1. Build and Start Server (in Docker)

```bash
cd rat
docker-compose up --build
```

Wait for logs to show:
```
🌍 PUBLIC URL: https://your-url.ngrok-free.dev
Server listening on 0.0.0.0:3000
```

### 2. Build Client

```bash
cd rat/rat-client
cargo build --release
```

### 3. Test Locally (Without Internet)

```bash
./target/release/rat-client http://localhost:3000
```

You should see:
```
🔗 Created session: abc-123-def-456
🔗 Connecting to remote shell...

[REMOTE] Connected!

bash-5.1$
```

Type commands:
```bash
bash-5.1$ pwd
/app
bash-5.1$ ls
Cargo.toml  src
bash-5.1$ echo "Hello from remote!"
Hello from remote!
bash-5.1$ cd /tmp
bash-5.1$ pwd
/tmp
```

Press Ctrl+D to exit.

### 4. Test Over Internet

Get public URL from docker logs:
```bash
docker-compose logs rat | grep "PUBLIC URL"
# Example output: https://your-unique-id.ngrok-free.dev
```

From ANY computer/phone with Rust client:
```bash
./target/release/rat-client https://your-unique-id.ngrok-free.dev
```

Should connect to the Docker container's shell over the internet!

### 5. Test Session Reconnection

```bash
# Start session, note the session ID
./target/release/rat-client https://your-url.ngrok-free.dev
# Output: Created session: abc-123-def-456

# Do some work
bash-5.1$ cd /tmp
bash-5.1$ touch test.txt
# Disconnect (Ctrl+D)

# Reconnect to same session
./target/release/rat-client https://your-url.ngrok-free.dev --session abc-123-def-456

# Still in /tmp!
bash-5.1$ pwd
/tmp
bash-5.1$ ls
test.txt
```

### 6. List and Stop Sessions

```bash
# List active sessions
curl https://your-url.ngrok-free.dev/sessions

# Stop a session
./target/release/rat-client https://your-url.ngrok-free.dev --stop abc-123-def-456
```

## What You Can Do

Everything you'd do in a normal terminal:
- Navigate directories (`cd`, `ls`, `pwd`)
- Edit files (`vim`, `nano`)
- Run programs (`python`, `node`, `cargo run`)
- Git operations (`git status`, `git commit`)
- View logs (`tail -f`, `less`)
- Interactive programs (they all work!)

## Troubleshooting

**Client can't connect:**
- Check server is running: `docker-compose ps`
- Verify ngrok is active: `curl http://localhost:4040/api/tunnels`
- Test local first: `./rat-client http://localhost:3000`

**Terminal looks weird:**
- PTY defaults to 80x24. Resize not implemented yet.
- Some programs expect specific TERM. PTY sets `TERM=xterm-256color`

**Session disconnects:**
- Sessions stay alive in server until explicitly stopped
- Reconnect with `--session <id>`
- Or create new session (old one still running in background)

## How It's Different from HTTP Commands

**Old way (HTTP POST /execute):**
```bash
curl -X POST /execute -d '{"command": "ls"}'
# Wait for response
# {"output": "file1 file2"}
# No state, each command isolated
```

**New way (WebSocket shell):**
```bash
./rat-client <url>
# Interactive shell, maintains state
ls
cd /tmp
pwd  # Shows /tmp because state persists
```

This is the SSH-like experience you wanted!
