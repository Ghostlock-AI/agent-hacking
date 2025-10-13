"""
Usage:
    python client.py              # interactive REPL
    python client.py "hello"      # single prompt

Environment:
    SERVER: override server URL (default http://localhost:8000/chat)
"""
import os
import sys
from uuid import uuid4

import httpx

SERVER = os.getenv("SERVER", "http://localhost:8000/chat")
THREAD_ID = os.getenv("THREAD_ID", str(uuid4()))


def stream_once(prompt: str) -> None:
    with httpx.stream("POST", SERVER, json={"message": prompt, "thread_id": THREAD_ID}, timeout=None) as r:
        r.raise_for_status()
        for chunk in r.iter_text():
            if chunk:
                print(chunk, end="", flush=True)
    print()


def main() -> None:
    if len(sys.argv) > 1:
        stream_once(" ".join(sys.argv[1:]))
        return

    try:
        while True:
            prompt = input("You: ").strip()
            if prompt.lower() in {"exit", "quit"}:
                break
            stream_once(prompt)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
