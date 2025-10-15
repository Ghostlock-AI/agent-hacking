# Build stage
FROM rust:1.83 as builder

WORKDIR /app

# Copy manifest files
COPY Cargo.toml ./

# Copy source code
COPY src ./src

# Build the application in release mode
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install necessary runtime dependencies including ngrok
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    wget \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install ngrok
RUN wget https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-linux-amd64.tgz && \
    tar -xvzf ngrok-v3-stable-linux-amd64.tgz && \
    mv ngrok /usr/local/bin/ngrok && \
    rm ngrok-v3-stable-linux-amd64.tgz && \
    chmod +x /usr/local/bin/ngrok

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/rat /usr/local/bin/rat

# Expose the default port
EXPOSE 3000

# Run the server with ngrok enabled
CMD ["rat", "--ngrok"]
