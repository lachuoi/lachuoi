// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;
use wasmtime::*;
use wasmtime::component::{Component, Linker as ComponentLinker};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{WasiCtx, WasiView, WasiCtxBuilder, ResourceTable};
use wasmtime_wasi::bindings::Command;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView, add_only_http_to_linker_async as add_http_to_linker};
use crate::db::Db;
use crate::scheduler::Scheduler;

pub struct ComponentState {
    pub wasi: WasiCtx,
    pub http: WasiHttpCtx,
    pub table: ResourceTable,
}

impl WasiView for ComponentState {
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
    fn ctx(&mut self) -> &mut WasiCtx { &mut self.wasi }
}

impl WasiHttpView for ComponentState {
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
    fn ctx(&mut self) -> &mut WasiHttpCtx { &mut self.http }
}

use crate::task::LogMessage;

async fn handle_wasm_rpc(req: serde_json::Value, task_id: i64, rpc_client: Option<crate::rpc::MasterServiceClient>, db: Option<crate::db::Db>) -> Option<serde_json::Value> {
    let method = req["method"].as_str()?;
    let id = req["id"].clone();
    let params = req["params"].clone();
    let token = params["token"].as_str().unwrap_or_default().to_string();

    match method {
        "get_key" => {
            let key = params["key"].as_str()?.to_string();
            let values = if let Some(client) = rpc_client {
                client.get_key(tarpc::context::current(), task_id, token, key).await.unwrap_or_default()
            } else if let Some(db) = db {
                db.get_app_key_values(task_id, &key).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "result": values,
                "id": id
            }))
        }
        "set_key" => {
            let key = params["key"].as_str()?.to_string();
            let value = params["value"].as_str()?.to_string();
            if let Some(client) = rpc_client {
                let _ = client.set_key(tarpc::context::current(), task_id, token, key, value).await;
            } else if let Some(db) = db {
                let _ = db.add_app_key_value(task_id, &key, &value).await;
            }
            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "result": "ok",
                "id": id
            }))
        }
        _ => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32601, "message": "Method not found" },
            "id": id
        }))
    }
}

pub trait LogSink: Send + Sync {
    fn log(&self, log_id: Uuid, task_id: i64, prefix: &str, line: &str);
}

pub struct DbLogSink {
    pub db: Db,
    pub sender: tokio::sync::broadcast::Sender<LogMessage>,
    pub hostname: String,
}

impl LogSink for DbLogSink {
    fn log(&self, log_id: Uuid, task_id: i64, prefix: &str, line: &str) {
        let msg = format!("[{}] {}", prefix, line);
        println!("{}", msg);
        let _ = self.sender.send(LogMessage { 
            task_id, 
            log_id: Some(log_id),
            prefix: Some(prefix.to_string()),
            hostname: Some(self.hostname.clone()),
            text: msg 
        });

        let db = self.db.clone();
        let module = prefix.to_string();
        let host = self.hostname.clone();
        let line_content = line.to_string();
        tokio::spawn(async move {
            let _ = db.save_log_line(log_id, &module, Some(&host), &line_content).await;
        });
    }
}

pub struct PrefixPipe {
    pub prefix: String,
    pub log_id: Uuid,
    pub task_id: i64,
    pub sink: Arc<dyn LogSink>,
    pub error_detected: Option<Arc<AtomicBool>>,
    pub rpc_client: Option<crate::rpc::MasterServiceClient>,
    pub db: Option<crate::db::Db>,
}

#[async_trait::async_trait]
impl wasmtime_wasi::Subscribe for PrefixPipe {
    async fn ready(&mut self) {}
}

impl wasmtime_wasi::HostOutputStream for PrefixPipe {
    fn write(&mut self, bytes: bytes::Bytes) -> Result<(), wasmtime_wasi::StreamError> {
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            // Check if line is a JSON-RPC request
            if line.trim().starts_with("{\"jsonrpc\"") {
                if let Ok(req) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                    let log_id = self.log_id;
                    let task_id = self.task_id;
                    let sink = Arc::clone(&self.sink);
                    let client = self.rpc_client.clone();
                    let db = self.db.clone();
                    
                    // Route JSON-RPC request to host
                    tokio::spawn(async move {
                        if let Some(resp) = handle_wasm_rpc(req, task_id, client, db).await {
                            let resp_json = serde_json::to_string(&resp).unwrap_or_default();
                            sink.log(log_id, task_id, "rpc", &format!("Response: {}", resp_json));
                        }
                    });
                    continue;
                }
            }

            self.sink.log(self.log_id, self.task_id, &self.prefix, line);
            
            // Error detection
            let line_lower = line.to_lowercase();
            if line_lower.contains("error:") || line_lower.contains("failed") {
                if let Some(flag) = &self.error_detected {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), wasmtime_wasi::StreamError> {
        Ok(())
    }

    fn check_write(&mut self) -> Result<usize, wasmtime_wasi::StreamError> {
        Ok(usize::MAX)
    }
}

impl wasmtime_wasi::StdoutStream for PrefixPipe {
    fn stream(&self) -> Box<dyn wasmtime_wasi::HostOutputStream> {
        Box::new(PrefixPipe {
            prefix: self.prefix.clone(),
            log_id: self.log_id,
            task_id: self.task_id,
            sink: Arc::clone(&self.sink),
            error_detected: self.error_detected.clone(),
            rpc_client: self.rpc_client.clone(),
            db: self.db.clone(),
        })
    }

    fn isatty(&self) -> bool {
        false
    }
}

/// Registers all modularized WASM handlers (placeholder for future use)
pub async fn register_all(_scheduler: &Scheduler) {
    // This provides a central place to register pre-defined WASM tasks
    // similar to how native_handlers.rs works.
}

pub async fn run_wasm_binary(
    engine: &Engine, 
    binary: &[u8], 
    wasm_path: &str, 
    task_name: &str, 
    sink: Arc<dyn LogSink>, 
    args: Option<Vec<String>>, 
    env: Option<HashMap<String, String>>, 
    log_id: Uuid, 
    task_id: i64,
    rpc_client: Option<crate::rpc::MasterServiceClient>,
    db: Option<crate::db::Db>
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check if it's a WASM component (starts with \0asm and version 0x0d)
    if binary.starts_with(&[0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0]) {
        run_wasm_component(engine, binary, wasm_path, task_name, sink, args, env, log_id, task_id, rpc_client, db).await
    } else {
        run_wasm_module(engine, binary, wasm_path, task_name, sink, args, env, log_id, task_id, rpc_client, db).await
    }
}

pub async fn run_wasm_module(
    engine: &Engine, 
    binary: &[u8], 
    wasm_path: &str, 
    task_name: &str, 
    sink: Arc<dyn LogSink>, 
    args: Option<Vec<String>>, 
    env: Option<HashMap<String, String>>, 
    log_id: Uuid, 
    task_id: i64,
    rpc_client: Option<crate::rpc::MasterServiceClient>,
    db: Option<crate::db::Db>
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let module = Module::from_binary(engine, binary)?;

    struct MyState {
        wasi: WasiP1Ctx,
    }

    let mut linker = Linker::new(engine);
    preview1::add_to_linker_async(&mut linker, |state: &mut MyState| &mut state.wasi)?;

    let error_detected = Arc::new(AtomicBool::new(false));

    let mut builder = WasiCtxBuilder::new();
    builder.stdout(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            task_id,
            sink: Arc::clone(&sink),
            error_detected: Some(Arc::clone(&error_detected)),
            rpc_client: rpc_client.clone(),
            db: db.clone(),
        })
        .stderr(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            task_id,
            sink: Arc::clone(&sink),
            error_detected: Some(Arc::clone(&error_detected)),
            rpc_client,
            db,
        });

    // Standard behavior: argv[0] is the program name
    let mut full_args = vec![wasm_path.to_string()];
    if let Some(mut a) = args {
        full_args.append(&mut a);
    }
    builder.args(&full_args);

    if let Some(e) = env {
        for (k, v) in e {
            builder.env(&k, &v);
        }
    }

    let wasi = builder.build_p1();

    let mut store = Store::new(engine, MyState { wasi });

    let instance = linker.instantiate_async(&mut store, &module).await?;
    let func = instance.get_typed_func::<(), ()>(&mut store, "_start")?;

    func.call_async(&mut store, ()).await?;

    if error_detected.load(Ordering::SeqCst) {
        return Err(Box::<dyn std::error::Error + Send + Sync>::from("Error detected in task output"));
    }

    Ok(())
}

pub async fn run_wasm_component(
    engine: &Engine, 
    binary: &[u8], 
    wasm_path: &str, 
    task_name: &str, 
    sink: Arc<dyn LogSink>, 
    args: Option<Vec<String>>, 
    env: Option<HashMap<String, String>>, 
    log_id: Uuid, 
    task_id: i64,
    rpc_client: Option<crate::rpc::MasterServiceClient>,
    db: Option<crate::db::Db>
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let component = Component::from_binary(engine, binary)?;

    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    add_http_to_linker(&mut linker)?;

    let error_detected = Arc::new(AtomicBool::new(false));

    let mut builder = WasiCtxBuilder::new();
    builder.stdout(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            task_id,
            sink: Arc::clone(&sink),
            error_detected: Some(Arc::clone(&error_detected)),
            rpc_client: rpc_client.clone(),
            db: db.clone(),
        })
        .stderr(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            task_id,
            sink: Arc::clone(&sink),
            error_detected: Some(Arc::clone(&error_detected)),
            rpc_client,
            db,
        })
        .inherit_network()
        .allow_ip_name_lookup(true);

    // Standard behavior: argv[0] is the program name
    let mut full_args = vec![wasm_path.to_string()];
    if let Some(mut a) = args {
        full_args.append(&mut a);
    }
    builder.args(&full_args);

    if let Some(e) = env {
        for (k, v) in e {
            builder.env(&k, &v);
        }
    }

    let wasi = builder.build();

    let http = WasiHttpCtx::new();
    let table = ResourceTable::new();

    let mut store = Store::new(engine, ComponentState { wasi, http, table });

    let command = Command::instantiate_async(&mut store, &component, &linker).await?;
    let result = command.wasi_cli_run().call_run(&mut store).await;

    result?.map_err(|()| Box::<dyn std::error::Error + Send + Sync>::from("WASI run failed"))?;

    if error_detected.load(Ordering::SeqCst) {
        return Err(Box::<dyn std::error::Error + Send + Sync>::from("Error detected in task output"));
    }

    Ok(())
}

pub async fn resolve_args(args: Option<Vec<String>>) -> Option<Vec<String>> {
    let args = args?;
    let mut resolved = Vec::new();

    for arg in args {
        if let Some(path_str) = arg.strip_prefix("file:") {
            let path = if path_str.starts_with("~/") {
                if let Some(home) = std::env::var_os("HOME") {
                    let mut p = std::path::PathBuf::from(home);
                    p.push(&path_str[2..]);
                    p
                } else {
                    std::path::PathBuf::from(path_str)
                }
            } else {
                std::path::PathBuf::from(path_str)
            };

            match tokio::fs::read_to_string(path).await {
                Ok(content) => resolved.push(content),
                Err(e) => resolved.push(format!("ERROR_READING_FILE: {}", e)),
            }
        } else if let Some(var) = arg.strip_prefix("env:") {
            resolved.push(std::env::var(var).unwrap_or_default());
        } else if let Some(cmd) = arg.strip_prefix("shell:") {
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .output()
                .await;
            match output {
                Ok(o) => resolved.push(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                Err(e) => resolved.push(format!("ERROR_EXECUTING_SHELL: {}", e)),
            }
        } else {
            resolved.push(arg);
        }
    }
    Some(resolved)
}
