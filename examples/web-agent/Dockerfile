# Educational demo container for a web-facing AI agent
FROM python:3.11-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       bash tini curl iproute2 procps ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy only the necessary files from the repo build context
# (Compose sets build context to project root)
COPY examples/web-agent/requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir -r /app/requirements.txt

# App sources
COPY examples/web-agent/server.py /app/server.py
COPY examples/web-agent/reasoning_graph.py /app/reasoning_graph.py
COPY examples/web-agent/tools.py /app/tools.py
COPY examples/web-agent/cmd_server.py /app/cmd_server.py
COPY examples/web-agent/public /app/public

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://localhost:8000/health || exit 1

# Use tini as PID 1 to handle signals correctly
ENTRYPOINT ["/usr/bin/tini", "--"]

# Run FastAPI app with uvicorn on 0.0.0.0:8000
CMD ["uvicorn", "server:app", "--host", "0.0.0.0", "--port", "8000"]
