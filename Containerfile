# --- Build Stage ---
FROM rust:1.81-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifest and lock for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy source to pre-build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

# Copy actual source and static assets
COPY src ./src
COPY web ./web
COPY plugins ./plugins
COPY cron.toml ./

# Build the real application
# We touch main.rs to ensure cargo knows it needs to rebuild after the dummy run
RUN touch src/main.rs && cargo build --release

# --- Runtime Stage ---
FROM debian:bookworm-slim

# Install runtime dependencies (ca-certificates for HTTPS/Turso)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/task-scheduler /app/task-scheduler

# Copy required runtime directories and files
COPY --from=builder /app/web /app/web
COPY --from=builder /app/plugins /app/plugins
COPY --from=builder /app/cron.toml /app/cron.toml

# Create a directory for the database to allow volume mounting
RUN mkdir /app/data
ENV TURSO_DATABASE_URL="/app/data/tasks.db"

# Expose the web server port
EXPOSE 3000

# Run the scheduler
CMD ["./task-scheduler"]
