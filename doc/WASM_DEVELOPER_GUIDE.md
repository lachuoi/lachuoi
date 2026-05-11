# La Chuoi - WASM Developer Guide

This guide explains how to build and run WebAssembly (WASM) tasks on the La Chuoi distributed runtime.

## 🚀 Getting Started

La Chuoi supports both **WASI Preview 1** modules and **WASI Preview 2** (Component Model) binaries.

### Environment Variables

Every WASM task is provided with the following system environment variables:

- `APP_ID`: The unique numeric ID of the task.
- `LACHUOI_TOKEN`: A one-time authentication token required for system RPC calls.
- `ENVIRONMENT`: The current system mode (`production` or `development`). Default is `production`.
- `RPC_ENDPOINT`: The HTTP URI of the JSON-RPC service (e.g., `https://lachuoi.example.com/api/rpc`).

## 🛠 System Interaction (JSON-RPC)

WASM tasks can interact with the La Chuoi host (Master node) by sending standard **JSON-RPC 2.0** requests via HTTP POST to the `RPC_ENDPOINT`.

### Request Format

All requests must be POSTed as JSON and include the `token` and `task_id` in the `params`.

### KV Store Access

#### 1. Set a Value (`kv_set`)

```json
{
  "jsonrpc": "2.0",
  "method": "kv_set",
  "params": {
    "token": "YOUR_LACHUOI_TOKEN",
    "task_id": YOUR_APP_ID,
    "key": "last_run_timestamp",
    "value": "2026-05-11T12:00:00Z"
  },
  "id": 1
}
```

#### 2. Get a Value (`kv_get`)

```json
{
  "jsonrpc": "2.0",
  "method": "kv_get",
  "params": {
    "token": "YOUR_LACHUOI_TOKEN",
    "task_id": YOUR_APP_ID,
    "key": "last_run_timestamp"
  },
  "id": 2
}
```

## 📦 Building your WASM

### Using Rust

We recommend using the `wasm32-wasip1` or `wasm32-wasip2` targets.

```bash
# Add the target
rustup target add wasm32-wasip1

# Build your task
cargo build --target wasm32-wasip1 --release
```

### Example Task (Rust)

```rust
use std::env;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app_id = env::var("APP_ID")?.parse::<i64>()?;
    let token = env::var("LACHUOI_TOKEN")?;
    let endpoint = env::var("RPC_ENDPOINT")?;

    println!("Hello from Task {}!", app_id);

    // Prepare JSON-RPC request
    let rpc_request = json!({
        "jsonrpc": "2.0",
        "method": "kv_set",
        "params": {
            "token": token,
            "task_id": app_id,
            "key": "status",
            "value": "active"
        },
        "id": 1
    });

    // Execute via HTTP POST
    let client = reqwest::Client::new();
    let response = client.post(&endpoint)
        .json(&rpc_request)
        .send()
        .await?;

    println!("Master response status: {}", response.status());
    Ok(())
}
```

## ⚖️ Runtime Constraints

- **Networking**: Components (Preview 2) have outbound HTTP access enabled by default.
- **Filesystem**: Tasks run with a private, temporary virtual filesystem unless specific mappings are provided in `cron.toml`.
- **Resources**: CPU and Memory usage are monitored and reported to the Master dashboard.

## 🛡 Security

- **Sandboxing**: Tasks run in a Wasmtime sandbox, isolated from the host and other tasks.
- **Integrity**: Always provide a `sha256` checksum in your `cron.toml` to ensure the runtime only executes verified binaries.
