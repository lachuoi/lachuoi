# Build the project (e.g., just build, just build -- --release)
build flags="":
    cargo build {{flags}}

# Run the project (e.g., just run, just run -- --release, just run -- -- --reload)
run flags="":
    cargo run {{flags}}

# Reload the configuration
reload:
    cargo run -- reload

# Initialize the database (local development)
db-init:
    sqlite3 tasks.db < schema.sql

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
