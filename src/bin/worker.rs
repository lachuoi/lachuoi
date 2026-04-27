use futures_util::{StreamExt, SinkExt};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::{HashMap, HashSet};
use tokio_tungstenite::{connect_async_with_config, tungstenite::protocol::{Message, WebSocketConfig}, tungstenite::client::IntoClientRequest};
use wasmtime::{Engine, Config};
use lachuoi::wasm_handlers::{self, LogSink};
use lachuoi::task::{LogMessage, MasterMessage, WorkerMessage, RunRequest, JsonRpcNotification, SystemMetrics};
use reqwest::header::HeaderValue;
use sha2::{Sha256, Digest};

struct WebSocketLogSink {
    tx: tokio::sync::mpsc::UnboundedSender<JsonRpcNotification>,
    hostname: String,
}

impl LogSink for WebSocketLogSink {
    fn log(&self, log_id: uuid::Uuid, task_id: uuid::Uuid, prefix: &str, line: &str) {
        let msg = format!("[{}] {}", prefix, line);
        println!("{}", msg);
        let log_msg = LogMessage {
            task_id,
            log_id: Some(log_id),
            prefix: Some(prefix.to_string()),
            hostname: Some(self.hostname.clone()),
            text: msg,
        };
        
        let rpc_msg = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "log".to_string(),
            params: serde_json::to_value(log_msg).unwrap(),
        };
        
        let _ = self.tx.send(rpc_msg);
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    
    // Fix for rustls 0.23+ CryptoProvider panic
    let _ = rustls::crypto::ring::default_provider().install_default();

    let master_url = std::env::var("LACHUOI_MASTER_WS_URL")
        .or_else(|_| std::env::var("MASTER_WS_URL"))
        .unwrap_or_else(|_| "ws://127.0.0.1:9130/ws/worker".to_string());
    
    let api_key = std::env::var("LACHUOI_API_KEY").unwrap_or_default();
    let wasm_cache = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
    let pending_wasms = Arc::new(RwLock::new(HashMap::<String, Vec<u8>>::new()));
    let (wasm_tx, _) = tokio::sync::broadcast::channel::<String>(32);
    
    println!("Master URL: {}", master_url);
    if api_key.is_empty() {
        println!("Warning: LACHUOI_API_KEY is not set.");
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

        match connect_async_with_config(request, Some(config), false).await {
            Ok((ws_stream, _)) => {
                println!("Connected to master!");
                let (mut write, mut read) = ws_stream.split();
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<JsonRpcNotification>();

                // Task to send messages to master (including pings and metrics)
                let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
                let mut metrics_interval = tokio::time::interval(std::time::Duration::from_secs(30));
                let tx_metrics = tx.clone();
                
                let send_task = tokio::spawn(async move {
                    let mut sys = sysinfo::System::new_all();
                    loop {
                        tokio::select! {
                            _ = ping_interval.tick() => {
                                if write.send(Message::Ping(Vec::new().into())).await.is_err() {
                                    break;
                                }
                            }
                            _ = metrics_interval.tick() => {
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
                                
                                let rpc_msg = JsonRpcNotification {
                                    jsonrpc: "2.0".to_string(),
                                    method: "metrics".to_string(),
                                    params: serde_json::to_value(metrics).unwrap(),
                                };
                                let _ = tx_metrics.send(rpc_msg);
                            }
                            msg = rx.recv() => {
                                if let Some(m) = msg {
                                    let json = serde_json::to_string(&m).unwrap();
                                    if write.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                });

                // Main loop to receive messages from master
                while let Some(res) = read.next().await {
                    match res {
                        Ok(msg) => {
                            if msg.is_ping() { continue; }
                            if let Message::Text(text) = msg {
                                match serde_json::from_str::<JsonRpcNotification>(&text) {
                                    Ok(rpc_msg) => {
                                        match rpc_msg.method.as_str() {
                                            "bootstrap" => {
                                                if let Ok(MasterMessage::Bootstrap(info)) = serde_json::from_value::<MasterMessage>(rpc_msg.params) {
                                                    println!("Received bootstrap info: {} bytes of config, {} WASM paths", info.config_toml.len(), info.wasm_paths.len());
                                                    
                                                    // Request all missing WASM files
                                                    for path in info.wasm_paths {
                                                        let req = JsonRpcNotification {
                                                            jsonrpc: "2.0".to_string(),
                                                            method: "get_wasm".to_string(),
                                                            params: serde_json::json!({ "path": path }),
                                                        };
                                                        let _ = tx.send(req);
                                                    }
                                                }
                                            },
                                            "wasm_begin" => {
                                                if let Ok(MasterMessage::WasmBegin { path, total_size }) = serde_json::from_value::<MasterMessage>(rpc_msg.params) {
                                                    println!("Starting reception of WASM file: {} ({} bytes)", path, total_size);
                                                    pending_wasms.write().await.insert(path, Vec::with_capacity(total_size));
                                                }
                                            },
                                            "wasm_chunk" => {
                                                if let Ok(MasterMessage::WasmChunk { path, chunk, offset: _ }) = serde_json::from_value::<MasterMessage>(rpc_msg.params) {
                                                    if let Ok(bytes) = hex::decode(chunk) {
                                                        if let Some(buf) = pending_wasms.write().await.get_mut(&path) {
                                                            buf.extend_from_slice(&bytes);
                                                        }
                                                    }
                                                }
                                            },
                                            "wasm_end" => {
                                                if let Ok(MasterMessage::WasmEnd { path }) = serde_json::from_value::<MasterMessage>(rpc_msg.params) {
                                                    if let Some(binary) = pending_wasms.write().await.remove(&path) {
                                                        println!("Completed reception of WASM file: {} ({} bytes)", path, binary.len());
                                                        wasm_cache.write().await.insert(path.clone(), binary);
                                                        let _ = wasm_tx.send(path);
                                                    }
                                                }
                                            },
                                            "run_task" => {
                                                if let Ok(task_data) = serde_json::from_value::<MasterMessage>(rpc_msg.params) {
                                                    if let MasterMessage::RunTask(req) = task_data {
                                                        let tx_clone = tx.clone();
                                                        let wasm_cache_clone = wasm_cache.clone();
                                                        let hostname_clone = hostname.clone();
                                                        let wasm_tx_clone = wasm_tx.clone();
                                                        tokio::spawn(async move {
                                                            execute_task(req, tx_clone, wasm_cache_clone, hostname_clone, wasm_tx_clone).await;
                                                        });
                                                    }
                                                }
                                            },
                                            _ => eprintln!("Unknown method from master: {}", rpc_msg.method),
                                        }
                                    },
                                    Err(e) => eprintln!("Failed to parse JSON-RPC notification: {}. Text: {}", e, text),
                                }
                            }
                        },
                        Err(e) => {
                            eprintln!("WebSocket error from master: {}", e);
                            break;
                        }
                    }
                }
                
                send_task.abort();
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
    tx: tokio::sync::mpsc::UnboundedSender<JsonRpcNotification>,
    wasm_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    hostname: String,
    wasm_tx: tokio::sync::broadcast::Sender<String>
) {
    let mut config = Config::new();
    config.async_support(true);
    config.wasm_component_model(true);
    let engine = Engine::new(&config).expect("Failed to create Wasmtime engine");

    // Notify master that task has started
    let started_msg = WorkerMessage::TaskStarted {
        task_id: req.task_id,
        task_name: req.task_name.clone(),
    };
    let _ = tx.send(JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "task_started".to_string(),
        params: serde_json::to_value(started_msg).unwrap(),
    });

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

    // If binary is missing or checksum failed, request it again and wait
    if binary.is_none() {
        println!("Requesting WASM from master due to missing or invalid cache: {}", req.wasm_path);
        let mut wasm_rx = wasm_tx.subscribe();

        let get_req = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "get_wasm".to_string(),
            params: serde_json::json!({ "path": req.wasm_path }),
        };
        let _ = tx.send(get_req);

        // Wait for the WASM file to be received (with timeout)
        let wait_result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            while let Ok(path) = wasm_rx.recv().await {
                if path == req.wasm_path {
                    return true;
                }
            }
            false
        }).await;

        match wait_result {
            Ok(true) => {
                let cache = wasm_cache.read().await;
                if let Some(bin) = cache.get(&req.wasm_path) {
                    if let Some(expected) = &req.expected_sha256 {
                        let mut hasher = Sha256::new();
                        hasher.update(bin);
                        let actual = hex::encode(hasher.finalize());

                        if actual == *expected {
                            binary = Some(bin.clone());
                        } else {
                            let err_msg = format!("SHA256 mismatch for re-downloaded WASM {}: expected {}, got {}. Aborting task.", req.wasm_path, expected, actual);
                            println!("{}", err_msg);
                            let result_msg = WorkerMessage::TaskResult {
                                task_id: req.task_id,
                                log_id: req.log_id,
                                success: false,
                                error: Some(err_msg),
                            };
                            let _ = tx.send(JsonRpcNotification {
                                jsonrpc: "2.0".to_string(),
                                method: "task_result".to_string(),
                                params: serde_json::to_value(result_msg).unwrap(),
                            });
                            return;
                        }
                    } else {
                        binary = Some(bin.clone());
                    }
                }
            },
            _ => {
                let err_msg = format!("Timed out or failed waiting for WASM file: {}", req.wasm_path);
                println!("{}", err_msg);
                let result_msg = WorkerMessage::TaskResult {
                    task_id: req.task_id,
                    log_id: req.log_id,
                    success: false,
                    error: Some(err_msg),
                };
                let _ = tx.send(JsonRpcNotification {
                    jsonrpc: "2.0".to_string(),
                    method: "task_result".to_string(),
                    params: serde_json::to_value(result_msg).unwrap(),
                });
                return;
            }
        }
    }

    let result = if let Some(binary) = binary {
        let sink = Arc::new(WebSocketLogSink { tx: tx.clone(), hostname: hostname.clone() });
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
        ).await
    } else {
        Err(format!("WASM binary not found: {}", req.wasm_path).into())
    };

    let (success, error) = match result {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    let result_msg = WorkerMessage::TaskResult {
        task_id: req.task_id,
        log_id: req.log_id,
        success,
        error,
    };

    let rpc_msg = JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "task_result".to_string(),
        params: serde_json::to_value(result_msg).unwrap(),
    };
    let _ = tx.send(rpc_msg);
}
