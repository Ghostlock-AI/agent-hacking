# RAT

### setup

```bash
touch .env

# add this in your .env
NGROK_AUTHTOKEN=your_token_from_ngrok_dashboard
```

NGROK allows you to make a program easily avaialable to the internet.
NGROK is how we will communicate with the RAT remotely over the internet.
You can make an ngrok account and get the `NGROK_AUTHTOKEN` here: https://dashboard.ngrok.com/get-started/your-authtoken
The container will pickup your `.env` if it is in the `rat` project base.

### running

```bash
docker compose up --build -d
```

Then go to the container and view the logs:

```bash
2025-10-15T22:54:08.338646Z  INFO rat: Starting ngrok tunnel on port 3000
2025-10-15T22:54:11.547432Z  INFO rat: 🌍 PUBLIC URL: https://nonfluctuating-evelyn-slushiest.ngrok-free.dev⁠
2025-10-15T22:54:11.547449Z  INFO rat: 🌍 Access your server from anywhere at: https://nonfluctuating-evelyn-slushiest.ngrok-free.dev⁠
2025-10-15T22:54:11.548195Z  INFO rat: Starting server on 0.0.0.0:3000
2025-10-15T22:54:11.548225Z  INFO rat: Server listening on 0.0.0.0:3000
2025-10-15T22:54:11.548226Z  INFO rat: Endpoints:
2025-10-15T22:54:11.548227Z  INFO rat:   GET  /health          - Health check
2025-10-15T22:54:11.548228Z  INFO rat:   POST /execute         - Execute command and return full output
2025-10-15T22:54:11.548230Z  INFO rat:   POST /execute/stream  - Execute command and stream output
2025-10-15T22:57:12.407155Z  INFO rat: Executing command: echo with args: Some(["Hello from internet"])
```

What on line 2 you can see that there is a `PUBLIC_URL`.
Whatever that URL is, you want to copy it and outside the container run:

```bash
curl -X POST https://nonfluctuating-evelyn-slushiest.ngrok-free.dev/execute \
    -H "Content-Type: application/json" \
    -d '{"command": "echo", "args": ["Hello from internet"]}'

# the result will be
{"success":true,"output":"Hello from internet\n","error":null}%
```

This proves that you can access the rat on the internet and that it can run commands.
