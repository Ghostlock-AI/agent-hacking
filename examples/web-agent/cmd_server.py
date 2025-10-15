"""
Simple PTY TCP daemon.

- Listens on a TCP port and spawns a PTY-backed shell per connection.
- Bridges PTY I/O over a tiny framed binary protocol.
  Frame format: [1 byte type][4 bytes big-endian length][payload]

Types:
  0x10 handshake: payload is JSON {token?, rows?, cols?, cmd?}
  0x00 data:      payload is raw bytes to/from PTY
  0x01 resize:    payload is 8 bytes: rows(uint32 BE) + cols(uint32 BE)
  0x02 exit:      length 0; indicates PTY/session closed
  0xFF error:     payload is UTF-8 error text

Environment:
  CMDD_HOST  (default 0.0.0.0)
  CMDD_PORT  (default 7070)
  CMDD_TOKEN (optional, if set must match client)
  CMDD_SHELL (optional, default $SHELL or bash/sh)
"""
from __future__ import annotations

import asyncio
import argparse
import json
import os
import pty
import fcntl
import termios
import struct
import shutil
import signal
import sys
import atexit
from typing import Optional


TYPE_DATA = 0x00
TYPE_RESZ = 0x01
TYPE_EXIT = 0x02
TYPE_HELO = 0x10
TYPE_ERR  = 0xFF


def _pack_frame(ftype: int, payload: bytes = b"") -> bytes:
    return bytes([ftype]) + struct.pack(">I", len(payload)) + payload


async def _read_frame(reader: asyncio.StreamReader) -> tuple[int, bytes]:
    header = await reader.readexactly(5)
    ftype = header[0]
    (length,) = struct.unpack(">I", header[1:5])
    payload = b""
    if length:
        payload = await reader.readexactly(length)
    return ftype, payload


def _set_winsize(fd: int, rows: int, cols: int) -> None:
    try:
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    except Exception:
        pass


async def handle_client(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    peer = writer.get_extra_info("peername")
    expected = os.getenv("CMDD_TOKEN")

    # Handshake
    try:
        ftype, payload = await asyncio.wait_for(_read_frame(reader), timeout=5)
    except Exception:
        try:
            writer.write(_pack_frame(TYPE_ERR, b"handshake timeout"))
            await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()
        return
    if ftype != TYPE_HELO:
        writer.write(_pack_frame(TYPE_ERR, b"expected handshake frame"))
        await writer.drain()
        writer.close()
        return
    try:
        hello = json.loads(payload.decode("utf-8", errors="replace"))
    except Exception:
        writer.write(_pack_frame(TYPE_ERR, b"invalid handshake json"))
        await writer.drain()
        writer.close()
        return

    token = hello.get("token")
    if expected and token != expected:
        writer.write(_pack_frame(TYPE_ERR, b"unauthorized"))
        await writer.drain()
        writer.close()
        return

    rows = int(hello.get("rows", 24))
    cols = int(hello.get("cols", 80))
    cmd = hello.get("cmd")
    if isinstance(cmd, str) and cmd:
        sh = shutil.which("bash") or shutil.which("sh") or "/bin/sh"
        cmd = [sh, "-lc", cmd]
    elif isinstance(cmd, list) and cmd:
        cmd = [str(x) for x in cmd]
    else:
        shell_path = os.environ.get("CMDD_SHELL") or os.environ.get("SHELL") or shutil.which("bash") or shutil.which("sh") or "/bin/sh"
        cmd = [shell_path, "-l"]

    # Spawn PTY child
    pid, master_fd = pty.fork()
    if pid == 0:
        os.environ.setdefault("TERM", "xterm-256color")
        try:
            os.execvp(cmd[0], cmd)
        except Exception:
            os._exit(127)

    loop = asyncio.get_running_loop()
    _set_winsize(master_fd, rows, cols)
    try:
        os.set_blocking(master_fd, False)
    except Exception:
        pass

    stop = asyncio.Event()

    def on_pty_readable() -> None:
        try:
            data = os.read(master_fd, 4096)
            if data:
                writer.write(_pack_frame(TYPE_DATA, data))
            else:
                loop.remove_reader(master_fd)
                if not stop.is_set():
                    stop.set()
        except OSError:
            try:
                loop.remove_reader(master_fd)
            except Exception:
                pass
            if not stop.is_set():
                stop.set()

    loop.add_reader(master_fd, on_pty_readable)

    async def pump_client() -> None:
        nonlocal rows, cols
        try:
            while True:
                ftype, payload = await _read_frame(reader)
                if ftype == TYPE_DATA:
                    try:
                        os.write(master_fd, payload)
                    except OSError:
                        break
                elif ftype == TYPE_RESZ:
                    if len(payload) == 8:
                        r, c = struct.unpack(">II", payload)
                        _set_winsize(master_fd, int(r), int(c))
                        rows, cols = int(r), int(c)
                elif ftype == TYPE_EXIT:
                    break
                else:
                    # Unknown control type; ignore
                    pass
        except Exception:
            pass
        finally:
            if not stop.is_set():
                stop.set()

    client_task = asyncio.create_task(pump_client())

    try:
        while not stop.is_set():
            await asyncio.wait({client_task}, timeout=0.05)
            await writer.drain()
    finally:
        try:
            loop.remove_reader(master_fd)
        except Exception:
            pass
        try:
            os.close(master_fd)
        except Exception:
            pass
        try:
            os.kill(pid, signal.SIGHUP)
        except Exception:
            pass
        try:
            writer.write(_pack_frame(TYPE_EXIT, b""))
            await writer.drain()
        except Exception:
            pass
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass


async def main() -> None:
    host = os.getenv("CMDD_HOST", "0.0.0.0")
    port = int(os.getenv("CMDD_PORT", "7070"))

    server = await asyncio.start_server(handle_client, host=host, port=port)
    addrs = ", ".join(str(sock.getsockname()) for sock in server.sockets or [])
    print(f"cmd daemon listening on {addrs}")
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    def parse_args(argv: list[str]) -> argparse.Namespace:
        ap = argparse.ArgumentParser(description="PTY TCP daemon")
        ap.add_argument("--daemon", action="store_true", help="Run in background and return immediately")
        ap.add_argument("--pidfile", help="Write daemon PID to this file (with --daemon)")
        return ap.parse_args(argv)

    def daemonize(pidfile: Optional[str] = None) -> None:
        # Double-fork to fully detach.
        try:
            pid = os.fork()
            if pid > 0:
                # Parent exits
                os._exit(0)
        except OSError:
            sys.exit(1)

        os.setsid()
        try:
            pid = os.fork()
            if pid > 0:
                os._exit(0)
        except OSError:
            sys.exit(1)

        # Redirect stdio to /dev/null
        sys.stdout.flush()
        sys.stderr.flush()
        with open(os.devnull, 'rb', 0) as devnull_in:
            os.dup2(devnull_in.fileno(), 0)
        with open(os.devnull, 'ab', 0) as devnull_out:
            os.dup2(devnull_out.fileno(), 1)
            os.dup2(devnull_out.fileno(), 2)

        if pidfile:
            def _cleanup() -> None:
                try:
                    os.remove(pidfile)
                except Exception:
                    pass
            try:
                with open(pidfile, 'w') as fh:
                    fh.write(str(os.getpid()))
                atexit.register(_cleanup)
            except Exception:
                pass

    args = parse_args(sys.argv[1:])
    try:
        if args.daemon:
            daemonize(args.pidfile)
        asyncio.run(main())
    except KeyboardInterrupt:
        # Exit code 130 indicates SIGINT-like exit
        sys.exit(130)
