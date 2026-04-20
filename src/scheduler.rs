use chrono::{Utc, DateTime};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;
use wasmtime::*;
use wasmtime::component::{Component, Linker as ComponentLinker};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{WasiCtx, WasiView, WasiCtxBuilder, ResourceTable, Subscribe};
use wasmtime_wasi::bindings::Command;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView, add_only_http_to_linker_async as add_http_to_linker};
use chrono_tz::Tz;
use sha2::{Sha256, Digest};
use std::io::BufReader;

struct ComponentState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
}

impl WasiView for ComponentState {
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
    fn ctx(&mut self) -> &mut WasiCtx { &mut self.wasi }
}

impl WasiHttpView for ComponentState {
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
    fn ctx(&mut self) -> &mut WasiHttpCtx { &mut self.http }
}

use crate::task::ScheduledTask;
use crate::db::Db;

use std::pin::Pin;
use std::future::Future;
// Type alias for task handlers - functions that execute when a task runs
type TaskHandler = Arc<dyn Fn(Uuid, Db, tokio::sync::broadcast::Sender<String>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

use std::sync::atomic::{AtomicBool, Ordering};

struct PrefixPipe {
    prefix: String,
    log_id: Uuid,
    sender: tokio::sync::broadcast::Sender<String>,
    db: Db,
    error_detected: Option<Arc<AtomicBool>>,
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
            tokio::spawn(async move {
                let _ = db.save_log_line(log_id, &msg).await;
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

pub struct Scheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, TaskHandler>>>,
    native_handlers: Arc<RwLock<HashMap<String, TaskHandler>>>,
    running: Arc<RwLock<bool>>,
    db: Db,
    wasm_engine: Engine,
    plugins_dir: std::path::PathBuf,
    log_sender: tokio::sync::broadcast::Sender<String>,
    status_sender: tokio::sync::broadcast::Sender<Vec<crate::task::TaskStatus>>,
}

impl Scheduler {
    pub fn new(db: Db) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        config.wasm_component_model(true);
        let wasm_engine = Engine::new(&config).expect("Failed to create Wasmtime engine");
        let (log_sender, _) = tokio::sync::broadcast::channel(100);
        let (status_sender, _) = tokio::sync::broadcast::channel(100);

        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            native_handlers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            db,
            wasm_engine,
            plugins_dir: std::path::PathBuf::from("plugins"),
            log_sender,
            status_sender,
        }
    }

    pub async fn add_native_handler(&self, name: &str, handler: TaskHandler) {
        self.native_handlers.write().await.insert(name.to_string(), handler);
    }

    pub async fn reload_from_file(&self, path: &str) -> Result<(), String> {
        let config = crate::config::AppConfig::load(path)
            .map_err(|e| format!("Failed to load config: {}", e))?;
        self.sync_with_config(&config).await
    }

    pub fn subscribe_logs(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.log_sender.subscribe()
    }

    pub fn subscribe_status(&self) -> tokio::sync::broadcast::Receiver<Vec<crate::task::TaskStatus>> {
        self.status_sender.subscribe()
    }

    pub fn set_plugins_dir(&mut self, path: &str) {
        self.plugins_dir = std::path::PathBuf::from(path);
    }

    pub fn get_db(&self) -> Db {
        self.db.clone()
    }

    fn calculate_sha256(path: &std::path::Path) -> Result<String, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file for checksum: {}", e))?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        
        use std::io::Read;
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).map_err(|e| format!("Failed to read file for checksum: {}", e))?;
            if count == 0 { break; }
            hasher.update(&buffer[..count]);
        }
        
        let result = hasher.finalize();
        Ok(result.iter().map(|b| format!("{:02x}", b)).collect())
    }

    // Get status of all tasks
    pub async fn get_tasks_status(&self) -> Vec<crate::task::TaskStatus> {
        let tasks = self.tasks.read().await;
        tasks.values()
            .map(|t| crate::task::TaskStatus {
                id: t.id,
                name: t.name.clone(),
                cron: t.cron_expr.clone(),
                timezone: t.timezone.to_string(),
                task_type: t.task_type.clone(),
                last_run: t.last_run.map(|dt| dt.to_rfc3339()),
                last_duration_ms: t.last_duration,
                last_failed: t.last_failed,
                enabled: t.enabled,
            })
            .collect()
    }

    // Sync tasks with configuration
    pub async fn sync_with_config(
        &self,
        config: &crate::config::AppConfig,
    ) -> Result<(), String> {
        let native_handlers = self.native_handlers.read().await;
        
        // 1. Map existing tasks by name and capture their state if we update
        let existing_tasks = {
            let tasks = self.tasks.read().await;
            tasks.values()
                .map(|t| (t.name.clone(), t.id))
                .collect::<HashMap<String, Uuid>>()
        };

        let mut config_names = std::collections::HashSet::new();

        // 2. Add or Update tasks from config
        for task_cfg in &config.tasks {
            config_names.insert(task_cfg.name.clone());
            
            let mut should_update = true;
            let mut existing_state = None;

            if let Some(id) = existing_tasks.get(&task_cfg.name) {
                let tasks = self.tasks.read().await;
                if let Some(existing) = tasks.get(id) {
                    // Check if config actually changed
                    if existing.config_equals(&task_cfg.cron, &task_cfg.timezone, &task_cfg.task_type, task_cfg.payload.as_deref(), task_cfg.args.as_deref(), task_cfg.sha256.as_deref()) {
                        should_update = false;
                    } else {
                        // Capture state to preserve it
                        existing_state = Some((
                            existing.last_run,
                            existing.last_duration,
                            existing.last_failed,
                            existing.enabled,
                        ));
                    }
                }
            }

            if !should_update {
                // For native tasks, we still need to ensure the handler is registered
                // because it might have been loaded from DB without a handler
                if task_cfg.task_type == "native" {
                    if let Some(id) = existing_tasks.get(&task_cfg.name) {
                        if let Some(handler) = native_handlers.get(&task_cfg.name) {
                            let handler = Arc::clone(handler);
                            self.register_handler(*id, move |log_id, db, log_sender| handler(log_id, db, log_sender)).await?;
                        }
                    }
                }
                continue;
            }

            let task_id = existing_tasks.get(&task_cfg.name).cloned().unwrap_or_else(Uuid::new_v4);

            match task_cfg.task_type.as_str() {
                "native" => {
                    if let Some(handler) = native_handlers.get(&task_cfg.name) {
                        let handler = Arc::clone(handler);
                        let mut task = ScheduledTask::new(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone)?;
                        task.id = task_id;
                        
                        // Preserve state if updating
                        if let Some((last_run, last_dur, last_failed, enabled)) = existing_state {
                            task.last_run = last_run;
                            task.last_duration = last_dur;
                            task.last_failed = last_failed;
                            task.enabled = enabled;
                        }

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "native", None, None, None, task.enabled).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        self.register_handler(task_id, move |log_id, db, log_sender| handler(log_id, db, log_sender)).await?;
                        println!("Updated native task '{}'", task_cfg.name);
                    } else {
                        println!("Warning: No native handler found for task '{}'", task_cfg.name);
                    }
                }
                "wasm" => {
                    if let Some(payload) = &task_cfg.payload {
                        let mut task = ScheduledTask::new_wasm(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone, payload, task_cfg.args.clone(), task_cfg.sha256.clone())?;
                        task.id = task_id;

                        // Preserve state if updating
                        if let Some((last_run, last_dur, last_failed, enabled)) = existing_state {
                            task.last_run = last_run;
                            task.last_duration = last_dur;
                            task.last_failed = last_failed;
                            task.enabled = enabled;
                        }

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "wasm", Some(payload), task_cfg.args.clone(), task_cfg.sha256.as_deref(), task.enabled).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        self.register_wasm_handler(task_id, payload.to_string(), task_cfg.name.clone(), task_cfg.args.clone(), task_cfg.sha256.clone()).await?;
                        println!("Updated WASM task '{}'", task_cfg.name);
                    }
                }
                _ => println!("Warning: Unknown task type '{}'", task_cfg.task_type),
            }
        }

        // 3. Remove tasks that are in memory but NOT in config
        let to_remove: Vec<Uuid> = existing_tasks
            .iter()
            .filter(|(name, _)| !config_names.contains(*name))
            .map(|(_, id)| *id)
            .collect();

        for id in to_remove {
            println!("Removing task {} (not in config)", id);
            self.remove_task(id).await;
        }

        Ok(())
    }

    // Load tasks from the database
    pub async fn load_tasks(&self) -> Result<Vec<(Uuid, String, String)>, String> {
        let db_tasks = self.db.get_tasks().await
            .map_err(|e| format!("Failed to load tasks from DB: {}", e))?;
        
        // Fetch latest run info from logs
        let latest_logs = self.db.get_latest_task_logs().await
            .unwrap_or_default();

        let mut tasks = self.tasks.write().await;
        let mut loaded = Vec::new();
        for (id, name, cron_expr, timezone_str, task_type, payload, args, sha256, enabled) in db_tasks {
            let timezone = timezone_str.parse::<Tz>()
                .map_err(|e| format!("Invalid timezone in DB: {}", e))?;

            match ScheduledTask::from_db(id, name.clone(), cron_expr, timezone, task_type.clone(), payload.clone(), args.clone(), sha256.clone(), enabled) {
                Ok(mut task) => {
                    // Apply historical state
                    if let Some((run_at_str, duration_ms)) = latest_logs.get(&id) {
                        if let Ok(run_at) = DateTime::parse_from_rfc3339(run_at_str) {
                            task.last_run = Some(run_at.with_timezone(&task.timezone));
                        }
                        task.last_duration = duration_ms.map(|d| d as u64);
                    }

                    tasks.insert(id, task);
                    loaded.push((id, name.clone(), task_type.clone()));

                    // If it's a WASM task, automatically register its handler
                    if task_type == "wasm" {
                        if let Some(path) = payload {
                            self.register_wasm_handler(id, path, name.clone(), args, sha256).await?;
                        }
                    }
                }
                Err(e) => println!("Warning: Failed to load task '{}': {}", name, e),
            }
        }
        println!("Loaded {} tasks from database", tasks.len());
        Ok(loaded)
    }

    async fn resolve_args(args: Option<Vec<String>>) -> Option<Vec<String>> {
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

    async fn register_wasm_handler(&self, task_id: Uuid, wasm_path: String, task_name: String, args: Option<Vec<String>>, expected_sha256: Option<String>) -> Result<(), String> {
        let engine = self.wasm_engine.clone();
        
        let binary = if wasm_path.starts_with("https://") {
            println!("Downloading WASM for task '{}' from {}...", task_name, wasm_path);
            let response = reqwest::get(&wasm_path).await
                .map_err(|e| format!("Failed to download WASM from {}: {}", wasm_path, e))?;
            
            if !response.status().is_success() {
                return Err(format!("Failed to download WASM from {}: Status {}", wasm_path, response.status()));
            }

            let bytes = response.bytes().await
                .map_err(|e| format!("Failed to read WASM response body: {}", e))?;
            
            bytes.to_vec()
        } else {
            let full_path = if std::path::Path::new(&wasm_path).is_absolute() {
                std::path::PathBuf::from(&wasm_path)
            } else {
                self.plugins_dir.join(&wasm_path)
            };
            
            tokio::fs::read(&full_path).await
                .map_err(|e| format!("Failed to read WASM file at {:?}: {}", full_path, e))?
        };

        // Verify SHA256
        if let Some(expected) = &expected_sha256 {
            let mut hasher = Sha256::new();
            hasher.update(&binary);
            let result = hasher.finalize();
            let actual = result.iter().map(|b| format!("{:02x}", b)).collect::<String>();
            
            if actual != *expected {
                return Err(format!("SHA256 mismatch for task '{}': expected {}, got {}", task_name, expected, actual));
            }
            println!("SHA256 verified for task '{}'", task_name);
        } else {
            println!("Warning: No SHA256 provided for task '{}'. This is insecure.", task_name);
        }

        let path_for_error = wasm_path.clone();

        let handler = move |log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>| {
            let engine = engine.clone();
            let name = task_name.clone();
            let args = args.clone();
            let binary = binary.clone();
            let err_path = path_for_error.clone();

            Box::pin(async move {
                let resolved_args = Self::resolve_args(args).await;
                if let Err(e) = Self::run_wasm_binary(&engine, &binary, &err_path, &name, log_sender, resolved_args, log_id, db).await {
                    let err_msg = format!("Error executing WASM task: {}", e);
                    eprintln!("{}", err_msg);
                    Err(err_msg)
                } else {
                    Ok(())
                }
            }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        };

        self.handlers.write().await.insert(task_id, Arc::new(handler));
        Ok(())
    }

    async fn run_wasm_binary(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check if it's a WASM component (starts with \0asm and version 0x0d)
        if binary.starts_with(&[0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0]) {
            Self::run_wasm_component(engine, binary, wasm_path, task_name, log_sender, args, log_id, db).await
        } else {
            Self::run_wasm_module(engine, binary, wasm_path, task_name, log_sender, args, log_id, db).await
        }
    }

    async fn run_wasm_module(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

        let wasi = builder.build_p1();

        let mut store = Store::new(engine, MyState { wasi });

        let instance = linker.instantiate_async(&mut store, &module).await?;
        let func = instance.get_typed_func::<(), ()>(&mut store, "_start")?;

        func.call_async(&mut store, ()).await?;

        if error_detected.load(Ordering::SeqCst) {
            return Err(Box::<dyn std::error::Error + Send + Sync>::from("Error detected in task output"));
        }

        Ok(())    }

    async fn run_wasm_component(engine: &Engine, binary: &[u8], wasm_path: &str, task_name: &str, log_sender: tokio::sync::broadcast::Sender<String>, args: Option<Vec<String>>, log_id: Uuid, db: Db) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

        Ok(())    }




    // Register a handler for a specific task ID
    pub async fn register_handler<F, Fut>(&self, task_id: Uuid, handler: F) -> Result<(), String>
    where
        F: Fn(Uuid, Db, tokio::sync::broadcast::Sender<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let wrapped_handler = move |log_id, db, log_sender| {
            let h = Arc::clone(&handler);
            Box::pin(async move { h(log_id, db, log_sender).await }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        };

        self.handlers
            .write()
            .await
            .insert(task_id, Arc::new(wrapped_handler));
        Ok(())
    }

    // Register a new native task
    pub async fn add_task<F, Fut>(
        &self,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        handler: F,
    ) -> Result<Uuid, String>
    where
        F: Fn(Uuid, Db, tokio::sync::broadcast::Sender<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let task = ScheduledTask::new(name, cron_expr, timezone)?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "native", None, None, None, true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.register_handler(task_id, handler).await?;

        println!("Registered native task '{}' [{}] with id {}", name, timezone, task_id);
        Ok(task_id)
    }

    // Register a new WASM task
    pub async fn add_wasm_task(
        &self,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        wasm_path: &str,
        args: Option<Vec<String>>,
        sha256: Option<String>,
    ) -> Result<Uuid, String> {
        let task = ScheduledTask::new_wasm(name, cron_expr, timezone, wasm_path, args.clone(), sha256.clone())?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "wasm", Some(wasm_path), args.clone(), sha256.as_deref(), true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.register_wasm_handler(task_id, wasm_path.to_string(), name.to_string(), args, sha256).await?;

        println!("Registered WASM task '{}' [{}] with id {}", name, timezone, task_id);
        Ok(task_id)
    }

    // Remove a task from the scheduler
    pub async fn remove_task(&self, task_id: Uuid) -> bool {
        let _ = self.db.remove_task(task_id).await;
        let task_removed = self.tasks.write().await.remove(&task_id).is_some();
        self.handlers.write().await.remove(&task_id);
        task_removed
    }

    // Enable or disable a task without removing it
    pub async fn set_task_enabled(&self, task_id: Uuid, enabled: bool) {
        let _ = self.db.update_task_enabled(task_id, enabled).await;
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.enabled = enabled;
        }
    }

    pub async fn broadcast_status(&self) {
        let status = self.get_tasks_status().await;
        for s in &status {
            if s.last_duration_ms.is_some() {
                // println!("DEBUG: Broadcasting task '{}' with duration {}ms", s.name, s.last_duration_ms.unwrap());
            }
        }
        let _ = self.status_sender.send(status);
    }

    // Start the scheduler - runs in the background
    pub async fn start(self: Arc<Self>) {
        *self.running.write().await = true;

        let scheduler = Arc::clone(&self);
        let self_arc = Arc::clone(&self);
        tokio::spawn(async move {
            // Check for tasks to run every second
            let mut ticker = interval(Duration::from_secs(1));

            println!("Scheduler background loop started");

            while *scheduler.running.read().await {
                ticker.tick().await;
                scheduler.tick(Arc::clone(&self_arc)).await;
                scheduler.broadcast_status().await;
            }

            println!("Scheduler background loop stopped");
        });
        
        println!("Scheduler started");
    }

    // Stop the scheduler gracefully
    pub async fn stop(&self) {
        *self.running.write().await = false;
    }

    // Check all tasks and run those that are due
    async fn tick(&self, self_arc: Arc<Self>) {
        let mut tasks_to_run = Vec::new();

        // Collect tasks that need to run
        {
            let mut tasks = self.tasks.write().await;
            for (id, task) in tasks.iter_mut() {
                if !task.enabled {
                    continue;
                }

                let now_tz = Utc::now().with_timezone(&task.timezone);

                if task.should_run() {
                    println!("Task '{}' triggered (last run was: {:?})", task.name, task.last_run);
                    task.last_run = Some(now_tz);
                    tasks_to_run.push((*id, task.name.clone()));
                }
            }
        }

        if !tasks_to_run.is_empty() {
            self_arc.broadcast_status().await;
        }

        // Execute handlers for due tasks
        let handlers = self.handlers.read().await;
        for (task_id, task_name) in tasks_to_run {
            if let Some(handler) = handlers.get(&task_id) {
                let handler = Arc::clone(handler);
                let db = self.db.clone();
                let tasks_ref = Arc::clone(&self.tasks);
                let name = task_name.clone();
                let log_sender = self.log_sender.clone();
                let self_arc_clone = Arc::clone(&self_arc);

                tokio::spawn(async move {
                    if let Ok(log_id) = db.log_execution_start(task_id).await {
                        let start = std::time::Instant::now();
                        let start_msg = format!("[{}] Starting task...", name);
                        println!("{}", start_msg);
                        let _ = db.save_log_line(log_id, &start_msg).await;
                        let _ = log_sender.send(start_msg);
                        
                        let result = handler(log_id, db.clone(), log_sender.clone()).await;
                        let is_failed = result.is_err();
                        
                        if let Err(e) = &result {
                            let err_msg = format!("[{}] Task failed: {}", name, e);
                            println!("{}", err_msg);
                            let _ = db.save_log_line(log_id, &err_msg).await;
                            let _ = log_sender.send(err_msg);
                        }
                        
                        let duration_ms = start.elapsed().as_millis() as u64;
                        let finish_msg = format!("[{}] Task finished in {}ms", name, duration_ms);
                        println!("{}", finish_msg);
                        let _ = db.save_log_line(log_id, &finish_msg).await;
                        let _ = log_sender.send(finish_msg);
                        
                        let _ = db.log_execution_finish(log_id, duration_ms).await;
                        
                        // Update in-memory duration and status
                        {
                            let mut tasks = tasks_ref.write().await;
                            if let Some(task) = tasks.get_mut(&task_id) {
                                task.last_duration = Some(duration_ms);
                                task.last_failed = is_failed;
                            }
                        }
                        // Broadcast update immediately
                        self_arc_clone.broadcast_status().await;
                    }
                });
            }
        }
    }
}
