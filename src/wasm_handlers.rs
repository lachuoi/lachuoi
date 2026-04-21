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

pub struct PrefixPipe {
    pub prefix: String,
    pub log_id: Uuid,
    pub sender: tokio::sync::broadcast::Sender<String>,
    pub db: Db,
    pub error_detected: Option<Arc<AtomicBool>>,
}

#[async_trait::async_trait]
impl wasmtime_wasi::Subscribe for PrefixPipe {
    async fn ready(&mut self) {}
}

impl wasmtime_wasi::HostOutputStream for PrefixPipe {
    fn write(&mut self, bytes: bytes::Bytes) -> Result<(), wasmtime_wasi::StreamError> {
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            let msg = format!("[{}] {}", self.prefix, line);
            println!("{}", msg);
            let _ = self.sender.send(msg.clone());
            
            // Error detection
            let line_lower = line.to_lowercase();
            if line_lower.contains("error:") || line_lower.contains("failed") {
                if let Some(flag) = &self.error_detected {
                    flag.store(true, Ordering::SeqCst);
                }
            }

            // Save to database
            let db = self.db.clone();
            let log_id = self.log_id;
            let module = self.prefix.clone();
            let line_content = line.to_string();
            tokio::spawn(async move {
                let _ = db.save_log_line(log_id, &module, &line_content).await;
            });
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
            sender: self.sender.clone(),
            db: self.db.clone(),
            error_detected: self.error_detected.clone(),
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

pub async fn run_wasm_binary(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check if it's a WASM component (starts with \0asm and version 0x0d)
    if binary.starts_with(&[0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0]) {
        run_wasm_component(engine, binary, wasm_path, task_name, log_sender, args, env, log_id, db).await
    } else {
        run_wasm_module(engine, binary, wasm_path, task_name, log_sender, args, env, log_id, db).await
    }
}

pub async fn run_wasm_module(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
            sender: log_sender.clone(),
            db: db.clone(),
            error_detected: Some(Arc::clone(&error_detected)),
        })
        .stderr(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            sender: log_sender,
            db,
            error_detected: Some(Arc::clone(&error_detected)),
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

pub async fn run_wasm_component(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let component = Component::from_binary(engine, binary)?;

    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    add_http_to_linker(&mut linker)?;

    let error_detected = Arc::new(AtomicBool::new(false));

    let mut builder = WasiCtxBuilder::new();
    builder.stdout(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            sender: log_sender.clone(),
            db: db.clone(),
            error_detected: Some(Arc::clone(&error_detected)),
        })
        .stderr(PrefixPipe {
            prefix: task_name.to_string(),
            log_id,
            sender: log_sender,
            db,
            error_detected: Some(Arc::clone(&error_detected)),
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
