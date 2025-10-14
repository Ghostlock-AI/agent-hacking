"""
Interactive shell client over WebSocket PTY.

Usage:
  python shell_client.py                 # connect to ws://localhost:8000/shell
  python shell_client.py --url ws://host:8000/shell --token ABC123
  python shell_client.py --cmd /bin/bash -l

Requires:
  pip install websockets

Notes:
  - Sends raw stdin bytes to server and renders PTY output.
  - Uses an init message with terminal size and optional command.
  - Sends resize events on SIGWINCH.
"""
from __future__ import annotations

import argparse
import asyncio
import json
import os
import signal
import struct
import sys
import termios
import fcntl
import tty
from typing import List, Tuple

try:
    import websockets
except ImportError as e:
    print("Missing dependency: websockets. Install with `pip install websockets`.")
    raise


def get_winsize(fd: int) -> Tuple[int, int]:
    try:
        packed = fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\x00" * 8)
        rows, cols, _, _ = struct.unpack("HHHH", packed)
        return (rows or 24), (cols or 80)
    except Exception:
        return 24, 80


async def run_client(url: str, token: str | None, cmd: List[str] | None) -> int:
    # Build connection headers
    headers = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    stdin_fd = sys.stdin.fileno()
    stdout = sys.stdout.buffer

    # Save and set raw mode on TTY
    orig_attrs = termios.tcgetattr(stdin_fd)
    tty.setraw(stdin_fd)

    rows, cols = get_winsize(stdin_fd)

    # Use a queue so add_reader can enqueue stdin bytes without awaiting
    input_q: asyncio.Queue[bytes] = asyncio.Queue()

    async with websockets.connect(url, extra_headers=headers, max_size=None) as ws:
        # Send init message
        init_msg = {"type": "init", "rows": rows, "cols": cols}
        if cmd:
            init_msg["cmd"] = cmd
        await ws.send(json.dumps(init_msg))

        loop = asyncio.get_running_loop()

        # Resize handler
        def on_winch() -> None:
            r, c = get_winsize(stdin_fd)
            try:
                loop.create_task(ws.send(json.dumps({"type": "resize", "rows": r, "cols": c})))
            except RuntimeError:
                pass

        try:
            loop.add_signal_handler(signal.SIGWINCH, on_winch)
        except (NotImplementedError, RuntimeError):
            # Signal handlers may not be available (e.g., on some platforms)
            pass

        # STDIN readiness -> enqueue bytes
        def on_stdin_readable() -> None:
            try:
                data = os.read(stdin_fd, 4096)
                if data:
                    input_q.put_nowait(data)
            except OSError:
                pass

        loop.add_reader(stdin_fd, on_stdin_readable)

        async def pump_stdin() -> None:
            while True:
                data = await input_q.get()
                try:
                    await ws.send(data)
                except Exception:
                    break

        async def pump_ws() -> None:
            while True:
                try:
                    msg = await ws.recv()
                except websockets.exceptions.ConnectionClosed:
                    break
                if isinstance(msg, (bytes, bytearray)):
                    try:
                        stdout.write(msg)
                        stdout.flush()
                    except Exception:
                        break
                else:
                    # Control frames in JSON (e.g., exit)
                    try:
                        payload = json.loads(msg)
                    except Exception:
                        continue
                    if isinstance(payload, dict) and payload.get("type") == "exit":
                        break

        stdin_task = asyncio.create_task(pump_stdin())
        ws_task = asyncio.create_task(pump_ws())

        done, pending = await asyncio.wait({stdin_task, ws_task}, return_when=asyncio.FIRST_COMPLETED)
        for t in pending:
            t.cancel()

    # Restore terminal mode
    termios.tcsetattr(stdin_fd, termios.TCSADRAIN, orig_attrs)
    return 0


def parse_args(argv: List[str]) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="Interactive shell client over WebSocket PTY")
    ap.add_argument("--url", default=os.getenv("SHELL_URL", "ws://localhost:8000/shell"), help="WebSocket URL of shell endpoint")
    ap.add_argument("--token", default=os.getenv("SHELL_TOKEN"), help="Auth token for server (SHELL_TOKEN)")
    ap.add_argument("--cmd", nargs=argparse.REMAINDER, help="Command to run inside (default: login shell)")
    return ap.parse_args(argv)


def main() -> None:
    args = parse_args(sys.argv[1:])
    try:
        asyncio.run(run_client(args.url, args.token, args.cmd if args.cmd else None))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()

