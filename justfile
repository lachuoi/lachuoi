# Copyright 2026 Seungjin Kim
# SPDX-License-Identifier: MIT OR Apache-2.0

# Build both lachuoi and lachuoi-worker
build flags="":
    cargo build {{ flags }}

# Run the master (lachuoi)
run flags="":
    just run-master {{ flags }}

# Run the master (lachuoi)
run-master flags="":
    cargo run --bin lachuoi {{ flags }}

# Run the worker (lachuoi-worker)
run-worker flags="":
    cargo run --bin lachuoi-worker {{ flags }}

# Run both locally (needs LACHUOI_MASTER_WS_URL for worker)
run-all:
    just run-master & sleep 2 && just run-worker

# Reload the configuration
reload:
    cargo run --bin lachuoi -- reload

# Initialize the database (local development)
db-init: migrate

# Run database migrations
migrate:
    cargo run --bin lachuoi -- migrate

# Run all checks (clippy, fmt, test)
check-all: fmt lint test

# Check for compilation errors
check:
    cargo check

# Run clippy for linting
lint:
    cargo clippy -- -D warnings

# Format the code
fmt:
    cargo fmt --all

# Run tests
test:
    cargo test

# Clean build artifacts
clean:
    cargo clean
    rm -f .scheduler.pid

# Tag and push a new release based on the version in Cargo.toml
release:
    #!/usr/bin/env bash
    VERSION=$(grep '^version =' Cargo.toml | cut -d '"' -f 2)
    echo "Releasing v$VERSION..."
    git tag -a "v$VERSION" -m "Release v$VERSION"
    git push origin "v$VERSION"
    git push origin main

tr:
    just build --release
    rsync -avhz cron.toml 1.c:~/app/lachuoi/
    rsync -avhz web 1.c:~/app/lachuoi/
    rsync -avhz plugins 1.c:~/app/lachuoi/
    rsync -avhz target/release/lachuoi 1.c:~/app/lachuoi/
    rsync -avhz target/release/lachuoi-worker 1.c:~/app/lachuoi/
    rsync -avhz target/release/lachuoi-worker 3.o:~/apps/lachuoi/
    rsync -avhz -e "ssh -q" target/release/lachuoi-worker freeshell.de:~/app/lachuoi/
    rsync -avhz target/release/lachuoi-worker 0.z:~/app/lachuoi/
