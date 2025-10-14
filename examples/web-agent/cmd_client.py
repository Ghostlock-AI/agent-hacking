"""
Interactive TCP client for cmd_server.py

Protocol frames: [1 byte type][4-byte BE length][payload]
Types:
  0x10 handshake: JSON {token?, rows?, cols?, cmd?}
  0x00 data: raw bytes to/from PTY
  0x01 resize: rows(uint32) + cols(uint32)
  0x02 exit: no payload
  0xFF error: text

Usage:
  python cmd_client.py --host 127.0.0.1 --port 7070 --token ABC --cmd /bin/bash -l
  python cmd_client.py --host 127.0.0.1 --port 7070
"""
from __future__ import annotations

import argparse
import asyncio
import fcntl
import json
import os
import signal
import struct
import sys
import termios
import tty
from typing import List, Tuple


TYPE_DATA = 0x00
TYPE_RESZ = 0x01
TYPE_EXIT = 0x02
TYPE_HELO = 0x10
TYPE_ERR  = 0xFF


def pack_frame(ftype: int, payload: bytes = b"") -> bytes:
    return bytes([ftype]) + struct.pack(">I", len(payload)) + payload


async def read_frame(reader: asyncio.StreamReader) -> tuple[int, bytes]:
    header = await reader.readexactly(5)
    ftype = header[0]
    (length,) = struct.unpack(">I", header[1:5])
    payload = b""
    if length:
        payload = await reader.readexactly(length)
    return ftype, payload


def get_winsize(fd: int) -> Tuple[int, int]:
    try:
        packed = fcntl.ioctl(fd, termios.TIOCGWINSZ, b"\x00" * 8)
        r, c, _, _ = struct.unpack("HHHH", packed)
        return (r or 24), (c or 80)
    except Exception:
        return 24, 80


async def run_client(host: str, port: int, token: str | None, cmd: List[str] | None) -> int:
    reader, writer = await asyncio.open_connection(host=host, port=port)

    stdin_fd = sys.stdin.fileno()
    stdout = sys.stdout.buffer

    # Raw mode
    orig_attrs = termios.tcgetattr(stdin_fd)
    tty.setraw(stdin_fd)

    rows, cols = get_winsize(stdin_fd)
    hello = {"rows": rows, "cols": cols}
    if token:
        hello["token"] = token
    if cmd:
        hello["cmd"] = cmd

    writer.write(pack_frame(TYPE_HELO, json.dumps(hello).encode("utf-8")))
    await writer.drain()

    # Handle window changes
    loop = asyncio.get_running_loop()

    def on_winch() -> None:
        r, c = get_winsize(stdin_fd)
        try:
            writer.write(bytes([TYPE_RESZ]) + struct.pack(">I", 8) + struct.pack(">II", int(r), int(c)))
        except Exception:
            pass

    try:
        loop.add_signal_handler(signal.SIGWINCH, on_winch)
    except (NotImplementedError, RuntimeError):
        pass

    # Queue for stdin bytes
    input_q: asyncio.Queue[bytes] = asyncio.Queue()

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
                writer.write(pack_frame(TYPE_DATA, data))
                await writer.drain()
            except Exception:
                break

    async def pump_socket() -> None:
        while True:
            try:
                ftype, payload = await read_frame(reader)
            except Exception:
                break
            if ftype == TYPE_DATA:
                try:
                    stdout.write(payload)
                    stdout.flush()
                except Exception:
                    break
            elif ftype == TYPE_ERR:
                try:
                    sys.stderr.write(payload.decode("utf-8", errors="replace") + "\n")
                    sys.stderr.flush()
                except Exception:
                    pass
            elif ftype == TYPE_EXIT:
                break
            else:
                # Ignore unknown types
                pass

    try:
        t1 = asyncio.create_task(pump_stdin())
        t2 = asyncio.create_task(pump_socket())
        done, pending = await asyncio.wait({t1, t2}, return_when=asyncio.FIRST_COMPLETED)
        for t in pending:
            t.cancel()
    finally:
        termios.tcsetattr(stdin_fd, termios.TCSADRAIN, orig_attrs)
        try:
            writer.write(pack_frame(TYPE_EXIT))
            await writer.drain()
        except Exception:
            pass
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass
    return 0


def parse_args(argv: List[str]) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="Interactive TCP shell client")
    ap.add_argument("--host", default=os.getenv("CMDD_HOST", "127.0.0.1"))
    ap.add_argument("--port", type=int, default=int(os.getenv("CMDD_PORT", "7070")))
    ap.add_argument("--token", default=os.getenv("CMDD_TOKEN"))
    ap.add_argument("--cmd", nargs=argparse.REMAINDER, help="Command to exec inside (default login shell)")
    return ap.parse_args(argv)


def main() -> None:
    args = parse_args(sys.argv[1:])
    try:
        asyncio.run(run_client(args.host, args.port, args.token, args.cmd if args.cmd else None))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()

