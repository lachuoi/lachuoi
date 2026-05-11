# La Chuoi - WASI Runtime & Service Framework

> [!NOTE]
> The original Spin Framework and GPLv4-based code have been moved to the `legacy-gpl-version` branch. This is a new implementation of La Chuoi built from scratch: a Wasmtime-based WASI runtime framework licensed under MIT/Apache 2.0.

Project LACHUOI is named after the Vietnamese word *lá chuối*, meaning "banana leaf."

<div align="center">
  <img src="screenshots/dark-theme.png" alt="Dashboard Vertical Layout" width="100%" style="border-radius: 12px; box-shadow: 0 4px 24px rgba(0,0,0,0.1); border: 1px solid #e2e8f0; margin-bottom: 1rem;">
  <p><em>Modern, responsive dashboard featuring real-time task monitoring and controls.</em></p>
</div>

A high-performance, distributed WASI runtime and task management engine built with Rust. Beyond traditional **cron-based scheduling**, La Chuoi serves as a comprehensive **WASI runtime environment** capable of hosting web services, processing webhooks, and executing sandboxed components across a **distributed Master/Worker architecture**.

## 🚀 Key Features

- **Distributed Architecture**: Decouple task scheduling (Master) from task execution (Worker).
- **WebSocket Communication**: Master and Workers communicate via persistent, real-time WebSocket connections protected by a secure **NODE_KEY**.
- **Hybrid Execution**: Run native Rust tasks on the Master or delegate secure, sandboxed WASM components to specialized Workers.
- **Web Services & Webhooks**: Integrated support for receiving and processing webhooks, enabling event-driven task execution.
- **Universal WASI Runtime**: Full support for WASI Preview 1 and the modern Component Model (Preview 2).
- **Security & Integrity**: Unified middleware for GitHub OAuth and **NODE_KEY** authentication. Mandatory SHA256 checksum verification on both Master and Worker nodes.
- **Cluster Monitoring**: Comprehensive event logging (JSON-RPC) and real-time status tracking via SSE.

### 🎨 Modern Dashboard & Theme Support
<div align="center">
  <img src="screenshots/dark-light-theme.png" alt="Dark and Light Theme Support" width="100%" style="border-radius: 12px; box-shadow: 0 4px 24px rgba(0,0,0,0.1); border: 1px solid #e2e8f0; margin-top: 1rem; margin-bottom: 1rem;">
</div>

- **Dark/Light Theme**: Built-in support for system-preferred or manual theme switching.
- **Real-time Monitoring**: Live execution logs, worker resource metrics, and task status updates via Server-Sent Events (SSE).
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
TURSO_DATABASE_URL="lachuoi.db"                  # Local path (Preferred)
# TURSO_DATABASE_URL="libsql://your-db.turso.io" # Remote Turso
# TURSO_AUTH_TOKEN="your-secret-token"           # Only for remote Turso

# GitHub OAuth
GITHUB_CLIENT_ID="your_client_id"
GITHUB_CLIENT_SECRET="your_client_secret"
GITHUB_REDIRECT_URL="https://your-domain.com/auth/github/callback"

# Environment
ENVIRONMENT="production"                        # development or production (default: production)

# Security
NODE_KEY="a-very-strong-secret-key"
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

# WASM Plugin Task (Local)
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

### 1. Start the Master (`lachuoi`)
The master handles scheduling, the database, and the web dashboard.
```bash
cargo run --release --bin lachuoi
```

### 2. Start the Worker (`lachuoi-worker`)
The worker connects to the master and executes WASM tasks.
```bash
export LACHUOI_MASTER_WS_URL="wss://your-master-node.com/ws/worker"
export NODE_KEY="your-very-strong-secret-key"
cargo run --release --bin lachuoi-worker
```

### Zero-Downtime Reload
If you modify `cron.toml`, you can reload the configuration without restarting the master:
```bash
./target/release/lachuoi reload
# or
just reload
```

### Systemd User Services
For production or long-running instances, you can use the provided systemd user service files:
```bash
mkdir -p ~/.config/systemd/user/
cp systemd/*.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now lachuoi lachuoi-worker
```
*See [systemd/README.md](systemd/README.md) for detailed instructions.*

---

## 🛠️ Development & Releases

The project uses `just` as a command runner for common tasks.

- **Build all**: `just build --release`
- **Database Init**: `just db-init` (requires `sqlite3`)
- **Run Master**: `just run-master`
- **Run Worker**: `just run-worker`
- **Check All**: `just check-all` (lint, fmt, test)

### Releasing a New Version
Version synchronization is automated between `Cargo.toml`, Git tags, and Container images:
1. Update `version` in `Cargo.toml`.
2. Run `just release`. This will tag the commit and push it to GitHub, triggering a container build for both Master and Worker.

---

## 🧩 Architecture

### Master/Worker Model
La Chuoi uses a **Master/Worker** architecture for scalability and isolation:
- **Master**: Responsible for task scheduling, persistent state (SQLite), GitHub OAuth, and the Web Dashboard. It acts as a WebSocket server.
- **Worker**: Lightweight instances that connect to the Master. They host the Wasmtime runtime and execute tasks on demand.
- **Real-time Metrics**: Workers stream resource usage (CPU, Memory, Disk) and task status back to the Master for live dashboard updates.

### Security & Authentication
- **Unified Middleware**: All administrative and API endpoints are protected by a unified authentication layer. It supports either a valid GitHub OAuth session, a secure **NODE_KEY** header, or a specialized **LACHUOI_TOKEN** for tasks.
- **WASM Integrity**: Before executing any WASM binary, the Worker verifies its SHA256 checksum against the expected value provided by the Master. If a mismatch is detected or the binary is missing, the Worker automatically re-downloads a fresh copy from the Master.

---

## 🖥️ Web Dashboard

Accessible at `http://localhost:9130` (default port).

- **Monitoring**: View all tasks, their schedules, and real-time status.
- **Worker Nodes**: Real-time overview of connected workers, including their system resource metrics and currently running tasks.
- **Cluster Logs**: A comprehensive audit trail of Master/Worker interactions (JSON-RPC), task triggers, and execution results.
- **Live Logs**: View execution output in real-time directly from the dashboard.
- **Webhook Monitor**: Track incoming webhook requests, including headers and payloads.

---

## 📦 Deployment

### Container Images
La Chuoi provides separate, optimized container images for Master and Worker nodes via GitHub Packages (GHCR).

**Master Node:**
```bash
podman run -p 9130:9130 --env-file .env ghcr.io/your-username/lachuoi:latest
```

**Worker Node:**
```bash
podman run --env LACHUOI_MASTER_WS_URL="wss://your-master.com/ws/worker" --env NODE_KEY="your-key" ghcr.io/your-username/lachuoi-worker:latest
```

You can also build them locally using the provided `Containerfile.master` and `Containerfile.worker`.

---

## 📄 License
MIT or Apache 2.0. Copyright (c) 2026 Seungjin Kim.
