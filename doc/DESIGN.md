# La Chuoi - Project Instructions & Conventions

This document outlines the architecture, coding standards, and workflows for **La Chuoi**, a distributed WASI runtime and task management engine.

## 🏗 Architecture

La Chuoi follows a **Master/Worker** architecture designed for scalability and secure task execution.

### Master (`src/main.rs`)
- **Role**: Central coordinator, scheduler, and API gateway.
- **Responsibilities**:
  - Manages task configurations (`cron.toml`) and persists state in libSQL/SQLite.
  - **Schema Management**: The Master node initializes the database from a flattened schema (`schema.sql.toml`). All tables are created from scratch during initial setup.
  - **Migration Authority**: The Master node is the *only* component authorized to initialize or update the database (via `just migrate` or `cargo run -- migrate`).
  - Hosts the Web Dashboard and handles GitHub OAuth authentication.
  - Acts as a WebSocket server for Worker nodes using **tarpc over WebSockets**.
  - Hosts the **Task JSON-RPC HTTP Gateway** (`/api/rpc`) for sandboxed modules.
  - Provides a JSON-RPC interface (via WebSocket/tarpc) for monitoring and control.
  - Serves WASM binaries to Workers with integrity verification.

### Worker (`src/bin/worker.rs`)
- **Role**: Execution engine for sandboxed tasks.
- **Responsibilities**:
  - **No Database Access**: Workers are strictly prohibited from accessing the database. They receive all necessary configuration and tasks from the Master.
  - Connects to the Master via WebSocket using a secure **NODE_KEY**.
  - Maintains a Wasmtime runtime to execute WASI (Preview 1 & 2) components.
  - Streams resource metrics (CPU, memory) and execution logs back to the Master via WebSocket.
  - Verifies WASM binary integrity using SHA256 checksums.

## 🛠 Tech Stack

- **Language**: Rust (Edition 2024)
- **Runtime**: Tokio (Async), Wasmtime (WASI)
- **Database**: libSQL (Turso) / SQLite. **Preferred: Local libSQL file (`lachuoi.db`) with WAL mode enabled.**
- **Environment**: Supports `ENVIRONMENT="development"` or `ENVIRONMENT="production"` (default) for tailored runtime behavior.
- **Web Framework**: Axum
- **Communication**: Bidirectional RPC using **tarpc over WebSockets** (tokio-tungstenite) with **bincode** serialization.
- **Authentication**: GitHub OAuth (User), NODE_KEY (Service-to-Service), LACHUOI_TOKEN (Task-to-Master).

## 📜 Core Conventions

### 1. Environment Variables
- Use a `.env` file for local configuration.
- **ENVIRONMENT**: Set to `production` (default) or `development`. Controls cookie security, logging verbosity, and runtime checks.
- **Note**: Shell environment variables (e.g., `TURSO_DATABASE_URL`) will override values in `.env`. Ensure your shell is clean if you intend to use the local `lachuoi.db`.

### 2. Task Authentication
- Each task run is assigned a unique, ephemeral `LACHUOI_TOKEN`.
- Tasks must provide this token and their `APP_ID` in the `params` of all JSON-RPC calls to the Master node.
- Tokens are strictly validated and expire immediately after the task completes.

### 3. Error Handling
- Use `anyhow::Result` for application-level error handling.
- Prefer context-rich errors: `.with_context(|| "Failed to ...")`.

### 4. Async & Concurrency
- Use `tokio` for all asynchronous operations.
- Avoid blocking the async executor; use `tokio::task::spawn_blocking` if necessary.

### 5. Coding Style
- Follow standard Rust formatting: `cargo fmt`.
- Address all clippy warnings: `cargo clippy -- -D warnings`.
- Prefer explicit composition over complex trait hierarchies.

### 6. Security
- Never log or commit secrets (API keys, OAuth secrets).
- Always verify SHA256 checksums for WASM binaries before execution.
- Use the unified authentication middleware for all administrative endpoints.

## 🔄 Development Workflows

We use `just` as a command runner. Refer to the `justfile` for a complete list of commands.

### Common Commands
- **Run Checks**: `just check-all` (Runs fmt, lint, and tests).
- **Start Master**: `just run` or `just run-master`.
- **Start Worker**: `just run-worker`.
- **Database Migrations**: `just migrate`.
- **Clean Environment**: `just clean`.
- **CLI Help**: `lachuoi --help` or `lachuoi-worker --help`.

### Task Configuration
Tasks are defined in `cron.toml`. The Master supports zero-downtime reloads of this configuration using the `reload` command:
```bash
./target/release/lachuoi reload
# or
./target/release/lachuoi --reload
```

## 📁 Project Structure

- `src/main.rs`: Master node entry point and CLI.
- `src/bin/worker.rs`: Worker node entry point.
- `src/scheduler.rs`: Core task scheduling logic (Master).
- `src/db.rs`: Database abstraction layer (libSQL).
- `src/web.rs` & `src/web/`: Web server and dashboard handlers.
- `src/task.rs`: Task execution and lifecycle management.
- `src/wasm_handlers.rs`: Wasmtime integration and WASI environment.
- `web/templates/`: Askama HTML templates for the dashboard.
- `web/assets/` & `web/css/`: Frontend assets.

## 🚀 Deployment

- **Containers**: Use `Containerfile.master` and `Containerfile.worker`.
- **Systemd**: Production deployments should use the service files provided in the `systemd/` directory.
