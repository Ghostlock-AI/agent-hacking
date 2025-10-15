# RAT - Remote Administration Tool

A lightweight Rust-based HTTP server that allows you to remotely execute commands on your computer from anywhere. Perfect for coding from your phone or any other device.

## Features

- **HTTP REST API** for command execution
- **Daemon mode** - runs in the background
- **Streaming output** - watch command output in real-time via Server-Sent Events
- **Docker support** - easy containerized deployment
- **Internet accessible** - via ngrok or Cloudflare Tunnel

## API Endpoints

### `GET /health`
Health check endpoint.

**Response:**
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

### `POST /execute`
Execute a command and return the full output once complete.

**Request:**
```json
{
  "command": "ls",
  "args": ["-la"],
  "working_dir": "/tmp"
}
```

**Response:**
```json
{
  "success": true,
  "output": "total 8\ndrwx------  3 user  wheel  96 Oct 15 11:00 .\n...",
  "error": null
}
```

### `POST /execute/stream`
Execute a command and stream output line-by-line via Server-Sent Events.

**Request:**
```json
{
  "command": "ping",
  "args": ["-c", "5", "google.com"]
}
```

**Response:** (Server-Sent Events)
```
data: stdout: PING google.com (142.250.80.46): 56 data bytes
data: stdout: 64 bytes from 142.250.80.46: icmp_seq=0 ttl=115 time=12.3 ms
data: exit_code: 0
```

## Quick Start

### Local Development

1. **Build and run:**
   ```bash
   cargo run
   ```

2. **Run in daemon mode:**
   ```bash
   cargo run -- --daemon
   ```

3. **Custom port:**
   ```bash
   cargo run -- --port 3000
   ```

### Docker Deployment

1. **Build and run with Docker Compose:**
   ```bash
   docker-compose up -d
   ```

2. **Build manually:**
   ```bash
   docker build -t rat .
   docker run -p 8080:8080 rat
   ```

3. **View logs:**
   ```bash
   docker-compose logs -f
   ```

## Making It Internet Accessible

### Option 1: ngrok (Easiest)

1. **Install ngrok:**
   ```bash
   brew install ngrok  # macOS
   # or download from https://ngrok.com
   ```

2. **Start the server:**
   ```bash
   cargo run
   # or
   docker-compose up -d
   ```

3. **Create tunnel:**
   ```bash
   ngrok http 8080
   ```

4. **Access your server:**
   ```
   ngrok will give you a URL like: https://abc123.ngrok.io
   ```

### Option 2: Cloudflare Tunnel (Free, More Permanent)

1. **Install cloudflared:**
   ```bash
   brew install cloudflare/cloudflare/cloudflared  # macOS
   # or download from https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/install-and-setup/installation/
   ```

2. **Authenticate:**
   ```bash
   cloudflared tunnel login
   ```

3. **Create tunnel:**
   ```bash
   cloudflared tunnel create rat-server
   ```

4. **Configure tunnel:**
   Create `~/.cloudflared/config.yml`:
   ```yaml
   tunnel: <TUNNEL-ID>
   credentials-file: /Users/<your-user>/.cloudflared/<TUNNEL-ID>.json

   ingress:
     - hostname: rat.yourdomain.com
       service: http://localhost:8080
     - service: http_status:404
   ```

5. **Run tunnel:**
   ```bash
   cloudflared tunnel run rat-server
   ```

6. **Access your server:**
   ```
   https://rat.yourdomain.com
   ```

## Testing

### Health Check
```bash
curl http://localhost:8080/health
```

### Execute Simple Command
```bash
curl -X POST http://localhost:8080/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "echo", "args": ["Hello World"]}'
```

### Execute with Working Directory
```bash
curl -X POST http://localhost:8080/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "pwd", "working_dir": "/tmp"}'
```

### Stream Output
```bash
curl -X POST http://localhost:8080/execute/stream \
  -H "Content-Type: application/json" \
  -d '{"command": "ping", "args": ["-c", "3", "google.com"]}'
```

### Remote Testing (with ngrok)
```bash
# Replace <your-ngrok-url> with your actual ngrok URL
curl -X POST https://<your-ngrok-url>/execute \
  -H "Content-Type: application/json" \
  -d '{"command": "ls", "args": ["-la"]}'
```

## Security Considerations

**WARNING:** This server executes arbitrary commands. Use with caution!

Recommendations:
1. **Add authentication** - Implement API key or JWT authentication
2. **Use HTTPS** - ngrok/Cloudflare provide this by default
3. **Firewall rules** - Restrict access by IP if possible
4. **Command allowlist** - Limit which commands can be executed
5. **Run in container** - Isolate execution environment
6. **Network isolation** - Consider VPN or private network

## Architecture

- **Web Framework**: Axum (high-performance async HTTP)
- **Async Runtime**: Tokio
- **Streaming**: Server-Sent Events (SSE)
- **CLI**: Clap
- **Daemon**: Daemonize
- **Logging**: Tracing

## Future Enhancements

- [ ] Authentication (API keys, JWT)
- [ ] Command history
- [ ] File upload/download
- [ ] Terminal session support (PTY)
- [ ] WebSocket support
- [ ] Rate limiting
- [ ] Command allowlist/blocklist
- [ ] Multiple user support

## License

MIT
