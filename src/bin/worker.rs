// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::{HashMap, HashSet};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use wasmtime;
use lachuoi::wasm_handlers::{self, LogSink};
use lachuoi::task::{LogMessage, RunRequest, SystemMetrics};
use reqwest::header::HeaderValue;
use sha2::{Sha256, Digest};

use lachuoi::rpc::{MasterServiceClient, WorkerService, WsTransport, multiplex};
use tarpc::server::{self, Channel};

#[derive(Clone)]
struct WorkerServer {
    tx: tokio::sync::mpsc::UnboundedSender<TaskCommand>,
}

enum TaskCommand {
    Run(RunRequest),
    Bootstrap { config_toml: String, wasm_paths: Vec<String> },
}

impl WorkerService for WorkerServer {
    async fn run_task(self, _: tarpc::context::Context, req: RunRequest) {
        let _ = self.tx.send(TaskCommand::Run(req));
    }

    async fn bootstrap(self, _: tarpc::context::Context, config_toml: String, wasm_paths: Vec<String>) {
        let _ = self.tx.send(TaskCommand::Bootstrap { config_toml, wasm_paths });
    }
}

struct RpcLogSink {
    client: MasterServiceClient,
    hostname: String,
}

impl LogSink for RpcLogSink {
    fn log(&self, log_id: uuid::Uuid, task_id: i64, prefix: &str, line: &str) {
        let msg = format!("[{}] {}", prefix, line);
        println!("{}", msg);
        let log_msg = LogMessage {
            task_id,
            log_id: Some(log_id),
            prefix: Some(prefix.to_string()),
            hostname: Some(self.hostname.clone()),
            text: msg,
        };
        
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.log(tarpc::context::current(), log_msg).await;
        });
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h" || args[1] == "help") {
        println!("La Chuoi Worker - Execution engine for sandboxed tasks");
        println!();
        println!("Usage: lachuoi-worker");
        println!();
        println!("Environment Variables:");
        println!("  LACHUOI_MASTER_WS_URL    WebSocket URL of the master node");
        println!("  NODE_KEY                 Shared secret key (must match Master)");
        println!("  ENVIRONMENT              'development' or 'production' (default: production)");
        return;
    }

    let env = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "production".to_string());
    println!("Starting La Chuoi Worker in {} mode...", env);
    
    // Fix for rustls 0.23+ CryptoProvider panic
    let _ = rustls::crypto::ring::default_provider().install_default();

    let master_url = std::env::var("LACHUOI_MASTER_WS_URL")
        .or_else(|_| std::env::var("MASTER_WS_URL"))
        .unwrap_or_else(|_| "ws://127.0.0.1:9130/ws/worker".to_string());
    
    let api_key = std::env::var("NODE_KEY").unwrap_or_default();
    let wasm_cache = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
    let (wasm_tx, _) = tokio::sync::broadcast::channel::<String>(32);
    
    println!("Master URL: {}", master_url);
    if api_key.is_empty() {
        println!("Warning: NODE_KEY is not set.");
    }
    
    loop {
        println!("Connecting to master...");
        
        let hostname = hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown".to_string());
        let mut request = master_url.as_str().into_client_request().unwrap();
        request.headers_mut().insert("X-API-Key", HeaderValue::from_str(&api_key).unwrap());
        request.headers_mut().insert("X-Worker-Hostname", HeaderValue::from_str(&hostname).unwrap());

        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(128 * 1024 * 1024);
        config.max_frame_size = Some(128 * 1024 * 1024);

        match tokio_tungstenite::connect_async_with_config(request, Some(config), false).await {
            Ok((ws_stream, _)) => {
                println!("Connected to master!");
                
                // Wrap WebSocket in tarpc-compatible transport
                let transport = WsTransport::new(
                    ws_stream,
                    |bin| tokio_tungstenite::tungstenite::Message::Binary(bin.into()),
                    |msg| {
                        if let tokio_tungstenite::tungstenite::Message::Binary(bin) = msg {
                            Some(bin.into())
                        } else {
                            None
                        }
                    }
                );
                
                // Multiplex bidirectional RPC
                // MasterService is variant A, WorkerService is variant B
                let (master_transport, worker_transport) = multiplex(transport);

                let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TaskCommand>();
                let worker_server = WorkerServer { tx: cmd_tx };
                let master_client = MasterServiceClient::new(tarpc::client::Config::default(), master_transport).spawn();

                // Task to report metrics
                let metrics_client = master_client.clone();
                let metrics_task = tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                    let mut sys = sysinfo::System::new_all();
                    loop {
                        interval.tick().await;
                        sys.refresh_cpu_all();
                        sys.refresh_memory();
                        
                        let disks = sysinfo::Disks::new_with_refreshed_list();
                        let mut disk_used = 0;
                        let mut disk_total = 0;
                        let mut seen_devices = HashSet::new();
                        
                        for disk in &disks {
                            let fs = disk.file_system().to_string_lossy();
                            if fs != "btrfs" || seen_devices.insert(disk.name().to_owned()) {
                                disk_used += disk.total_space() - disk.available_space();
                                disk_total += disk.total_space();
                            }
                        }
                        
                        let load_avg = sysinfo::System::load_average();
                        
                        let metrics = SystemMetrics {
                            cpu_usage: sys.global_cpu_usage(),
                            memory_used: sys.used_memory(),
                            memory_total: sys.total_memory(),
                            disk_used,
                            disk_total,
                            uptime: sysinfo::System::uptime(),
                            load_avg_one: Some(load_avg.one),
                            load_avg_five: Some(load_avg.five),
                            load_avg_fifteen: Some(load_avg.fifteen),
                        };
                        
                        if metrics_client.report_metrics(tarpc::context::current(), metrics).await.is_err() {
                            break;
                        }
                    }
                });

                // Task to handle incoming commands (RunTask)
                let master_client_for_tasks = master_client.clone();
                let wasm_cache_clone = wasm_cache.clone();
                let hostname_clone = hostname.clone();
                let wasm_tx_clone = wasm_tx.clone();
                let cmd_task = tokio::spawn(async move {
                    while let Some(cmd) = cmd_rx.recv().await {
                        match cmd {
                            TaskCommand::Run(req) => {
                                let client = master_client_for_tasks.clone();
                                let cache = wasm_cache_clone.clone();
                                let h = hostname_clone.clone();
                                let wtx = wasm_tx_clone.clone();
                                tokio::spawn(async move {
                                    execute_task(req, client, cache, h, wtx).await;
                                });
                            }
                            TaskCommand::Bootstrap { config_toml, wasm_paths } => {
                                println!("Received bootstrap info: {} bytes of config, {} WASM paths", config_toml.len(), wasm_paths.len());
                                // Request all missing WASM files
                                let client = master_client_for_tasks.clone();
                                let cache = wasm_cache_clone.clone();
                                for path in wasm_paths {
                                    let c = client.clone();
                                    let cache_clone = cache.clone();
                                    tokio::spawn(async move {
                                        if let Ok(Some(bin)) = c.get_wasm(tarpc::context::current(), path.clone()).await {
                                            cache_clone.write().await.insert(path, bin);
                                        }
                                    });
                                }
                            }
                        }
                    }
                });

                // Run the RPC server for this connection
                server::BaseChannel::with_defaults(worker_transport)
                    .execute(worker_server.serve())
                    .for_each(|f| async move { f.await })
                    .await;
                
                metrics_task.abort();
                cmd_task.abort();
                println!("Disconnected from master. Retrying in 5 seconds...");
            }
            Err(e) => {
                eprintln!("Failed to connect: {}. Retrying in 5 seconds...", e);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn execute_task(
    req: RunRequest,
    client: MasterServiceClient,
    wasm_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    hostname: String,
    _wasm_tx: tokio::sync::broadcast::Sender<String>
) {
    let mut config = wasmtime::Config::new();
    config.async_support(true);
    config.wasm_component_model(true);
    let engine = wasmtime::Engine::new(&config).expect("Failed to create Wasmtime engine");

    // Notify master that task has started
    let _ = client.task_started(tarpc::context::current(), req.task_id, req.task_name.clone()).await;

    let mut binary = {
        let cache = wasm_cache.read().await;
        if let Some(bin) = cache.get(&req.wasm_path) {
            // Verify SHA256 if provided
            if let Some(expected) = &req.expected_sha256 {
                let mut hasher = Sha256::new();
                hasher.update(bin);
                let actual = hex::encode(hasher.finalize());

                if actual == *expected {
                    Some(bin.clone())
                } else {
                    println!("SHA256 mismatch for cached WASM {}: expected {}, got {}. Re-requesting...", req.wasm_path, expected, actual);
                    None
                }
            } else {
                Some(bin.clone())
            }
        } else {
            None
        }
    };

    // If binary is missing or checksum failed, request it again
    if binary.is_none() {
        println!("Requesting WASM from master: {}", req.wasm_path);
        match client.get_wasm(tarpc::context::current(), req.wasm_path.clone()).await {
            Ok(Some(bin)) => {
                if let Some(expected) = &req.expected_sha256 {
                    let mut hasher = Sha256::new();
                    hasher.update(&bin);
                    let actual = hex::encode(hasher.finalize());

                    if actual == *expected {
                        binary = Some(bin.clone());
                        wasm_cache.write().await.insert(req.wasm_path.clone(), bin);
                    } else {
                        let err_msg = format!("SHA256 mismatch for downloaded WASM {}: expected {}, got {}.", req.wasm_path, expected, actual);
                        let _ = client.task_result(tarpc::context::current(), req.task_id, req.log_id, false, Some(err_msg)).await;
                        return;
                    }
                } else {
                    binary = Some(bin.clone());
                    wasm_cache.write().await.insert(req.wasm_path.clone(), bin);
                }
            },
            _ => {
                let err_msg = format!("Failed to download WASM file: {}", req.wasm_path);
                let _ = client.task_result(tarpc::context::current(), req.task_id, req.log_id, false, Some(err_msg)).await;
                return;
            }
        }
    }

    let result = if let Some(binary) = binary {
        let sink = Arc::new(RpcLogSink { client: client.clone(), hostname: hostname.clone() });
        wasm_handlers::run_wasm_binary(
            &engine,
            &binary,
            &req.wasm_path,
            &req.task_name,
            sink,
            req.args,
            req.env,
            req.log_id,
            req.task_id,
            Some(client.clone()),
            None,
        ).await
    } else {
        Err(format!("WASM binary not found: {}", req.wasm_path).into())
    };

    let (success, error) = match result {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    let _ = client.task_result(tarpc::context::current(), req.task_id, req.log_id, success, error).await;
}
