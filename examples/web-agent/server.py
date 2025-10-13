import os
import uuid
from typing import AsyncIterator

from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException
from fastapi.responses import PlainTextResponse, StreamingResponse
from langchain_openai import ChatOpenAI
from reasoning_graph import build_reasoning_graph

# Load .env copied into the image at build time
load_dotenv()
api_key = os.getenv("OPENAI_API_KEY")
if not api_key:
    raise RuntimeError("OPENAI_API_KEY not set; ensure .env is provided at build time")


# ---------- LangGraph plumbing ----------------------------------------------
LLM = ChatOpenAI(model="gpt-4o", temperature=0.2, streaming=True, openai_api_key=api_key)
GRAPH = build_reasoning_graph(LLM)

# ---------- FastAPI -----------------------------------------------------------
app = FastAPI(title="Web Agent Demo", version="0.1.0")


@app.get("/", response_class=PlainTextResponse)
def index() -> str:
    return (
        "Web Agent Demo is running.\n"
        "POST /chat with {'message': '...'} to stream a response.\n"
        "GET /health for health check.\n"
    )


@app.get("/health", response_class=PlainTextResponse)
def health() -> str:
    return "ok"


async def _token_stream(user_message: str) -> AsyncIterator[str]:
    last = ""
    try:
        for event in GRAPH.stream({"messages": [{"role": "user", "content": user_message}]}, stream_mode="values", config={"configurable": {"thread_id": "default"}}):
            if not isinstance(event, dict):
                continue
            messages = event.get("messages")
            if not messages:
                continue
            last_msg = messages[-1]
            content = getattr(last_msg, "content", None)
            if content:
                text = str(content)
                if len(text) >= len(last) and text.startswith(last):
                    delta = text[len(last):]
                else:
                    delta = text
                last = text
                if delta:
                    yield delta
    except Exception as e:
        yield f"\n[ERROR] {e}\n"


@app.post("/chat")
async def chat(payload: dict):
    msg = payload.get("message", "")
    thread_id = payload.get("thread_id") or "default"
    if not msg:
        raise HTTPException(422, "message field required")

    async def _stream() -> AsyncIterator[str]:
        last = ""
        try:
            for event in GRAPH.stream(
                {"messages": [{"role": "user", "content": msg}]},
                stream_mode="values",
                config={"configurable": {"thread_id": str(thread_id)}},
            ):
                if not isinstance(event, dict):
                    continue
                messages = event.get("messages")
                if not messages:
                    continue
                last_msg = messages[-1]
                content = getattr(last_msg, "content", None)
                if content:
                    text = str(content)
                    if len(text) >= len(last) and text.startswith(last):
                        delta = text[len(last):]
                    else:
                        delta = text
                    last = text
                    if delta:
                        yield delta
        except Exception as e:
            yield f"\n[ERROR] {e}\n"

    return StreamingResponse(_stream(), media_type="text/plain")


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("server:app", host="0.0.0.0", port=8000)
