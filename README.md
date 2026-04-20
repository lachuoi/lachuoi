# Cron Task Scheduler

A high-performance, distributed task management engine built with Rust. This scheduler supports both **native Rust handlers** and **sandboxed WASM plugins**, featuring a real-time web dashboard with GitHub OAuth security and persistent execution history.

## 🚀 Key Features

- **Hybrid Execution**: Run native Rust tasks or secure, sandboxed WASM components.
- **WASM Security**: Mandatory SHA256 checksum verification for all WASM binaries.
- **Modern Dashboard**: Responsive UI built with Tailwind CSS and Inter typography.
- **Real-time Monitoring**: Live execution logs and status updates via Server-Sent Events (SSE).
- **Persistent State**: Database-backed sessions and execution history (Turso/libSQL).
- **Zero-Downtime Reloads**: Hot-reload `cron.toml` configuration without stopping the service.
- **Interactive Controls**: Enable, disable, and sort tasks directly from the web UI.

---

## 🛠️ Prerequisites

- **Rust**: Latest stable version (1.81+ recommended).
- **Database**: A Turso account or a local libSQL/SQLite file.
- **GitHub OAuth App**: For secure dashboard authentication.

---

## ⚙️ Configuration

### 1. Environment Variables
Create a `.env` file in the root directory:

```bash
# Database Configuration
TURSO_DATABASE_URL="libsql://your-db.turso.io" # Or local path: "tasks.db"
TURSO_AUTH_TOKEN="your-secret-token"           # Only for remote Turso

# GitHub OAuth
GITHUB_CLIENT_ID="your_client_id"
GITHUB_CLIENT_SECRET="your_client_secret"
GITHUB_REDIRECT_URL="https://your-domain.com/auth/github/callback"
```

### 2. Task Configuration (`cron.toml`)
Define your tasks in `cron.toml`. Changes can be reloaded at runtime.

```toml
# Native Rust Task
[[task]]
name = "heartbeat"
cron = "0 * * * * *"
timezone = "UTC"
type = "native"

# WASM Plugin Task
[[task]]
name = "weather-station"
cron = "0 */10 * * * *"
timezone = "Asia/Seoul"
type = "wasm"
payload = "weather.wasm"
sha256 = "ad677d5c7c136f862aed95f61879d0b0bb80cfb6f9921..."
args = ["--city", "Seoul"]
```

---

## 🏃 Running the Application

### Build and Run
```bash
cargo run --release
```

### Zero-Downtime Reload
If you modify `cron.toml`, you can reload the configuration without restarting the service:
```bash
./task-scheduler reload
```
*This sends a SIGHUP signal to the main process via its PID file.*

---

## 🧩 Architecture

### Native Handlers
Native tasks are modularized in `src/native_handlers.rs`. To add a new native task:
1. Implement the logic in `src/native_handlers.rs`.
2. Register it in the `register_all` function.
3. Add a corresponding entry in `cron.toml`.

### WASM Plugins
WASM tasks run in a strictly sandboxed environment using **Wasmtime**.
- **SHA256 Check**: The scheduler verifies the binary hash before every execution.
- **Argument Resolution**: Supports dynamic argument injection (e.g., `file:~/.ssh/id_ed25519`).
- **Standard Out**: Logs are captured via `PrefixPipe` and streamed to the UI in real-time.

---

## 🖥️ Web Dashboard

Accessible at `http://localhost:9130` (default port).

- **Sorting**: Click any column header (Name, Type, Last Run, etc.) to sort. "Last Run" uses chronological date sorting.
- **Controls**: Use the **Enable/Disable** buttons to pause tasks without removing them from config.
- **Live Logs**: View the last 1000 lines of execution logs in the realtime console.

---

## 📦 Deployment

The project includes a `Containerfile` for Docker/Podman deployment:
```bash
podman build -t task-scheduler .
podman run -p 3000:3000 --env-file .env task-scheduler
```

---

## 📄 License
Internal proprietary engine. All rights reserved.
