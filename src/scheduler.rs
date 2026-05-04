// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use wasmtime::*;
use chrono_tz::Tz;
use sha2::{Sha256, Digest};
use std::io::BufReader;

use crate::task::{ScheduledTask, LogMessage};
use crate::db::Db;
use crate::wasm_handlers;

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;
use chrono::{Utc, DateTime};

use std::pin::Pin;
use std::future::Future;

// Type alias for task handlers - functions that execute when a task runs
type TaskHandler = Arc<dyn Fn(Uuid, Uuid, Db, tokio::sync::broadcast::Sender<LogMessage>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

pub struct Scheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, TaskHandler>>>,
    native_handlers: Arc<RwLock<HashMap<String, TaskHandler>>>,
    running: Arc<RwLock<bool>>,
    db: Db,
    wasm_engine: Engine,
    plugins_dir: std::path::PathBuf,
    log_sender: tokio::sync::broadcast::Sender<LogMessage>,
    status_sender: tokio::sync::broadcast::Sender<Vec<crate::task::TaskStatus>>,
    webhook_sender: tokio::sync::broadcast::Sender<crate::db::WebhookLog>,
    pub worker_tx: tokio::sync::broadcast::Sender<crate::task::MasterMessage>,
    worker_sender: tokio::sync::broadcast::Sender<Vec<crate::task::WorkerInfo>>,
    num_workers: Arc<std::sync::atomic::AtomicUsize>,

    wasm_cache: Arc<RwLock<HashMap<String, Arc<Vec<u8>>>>>,
    config_toml: Arc<RwLock<Option<String>>>,
    api_key: Option<String>,
    workers: Arc<RwLock<HashMap<Uuid, crate::task::WorkerInfo>>>,
    worker_channels: Arc<RwLock<HashMap<Uuid, tokio::sync::mpsc::UnboundedSender<crate::task::MasterMessage>>>>,
    task_to_worker: Arc<RwLock<HashMap<Uuid, String>>>,
}

impl Scheduler {
    pub fn new(db: Db) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        config.wasm_component_model(true);
        let wasm_engine = Engine::new(&config).expect("Failed to create Wasmtime engine");
        let (log_sender, _) = tokio::sync::broadcast::channel(100);
        let (status_sender, _) = tokio::sync::broadcast::channel(100);
        let (webhook_sender, _) = tokio::sync::broadcast::channel(100);
        let (worker_tx, _) = tokio::sync::broadcast::channel(100);
        let (worker_sender, _) = tokio::sync::broadcast::channel(100);

        let api_key = std::env::var("LACHUOI_API_KEY").ok();

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
            webhook_sender,
            worker_tx,
            worker_sender,
            num_workers: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            wasm_cache: Arc::new(RwLock::new(HashMap::new())),
            config_toml: Arc::new(RwLock::new(None)),
            api_key,
            workers: Arc::new(RwLock::new(HashMap::new())),
            worker_channels: Arc::new(RwLock::new(HashMap::new())),
            task_to_worker: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn verify_api_key(&self, key: &str) -> bool {
        match &self.api_key {
            Some(expected) => expected == key,
            None => true, // If no key set, allow all
        }
    }

    pub fn num_workers(&self) -> usize {
        self.num_workers.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_num_workers(&self, n: usize) {
        self.num_workers.store(n, std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn add_worker(&self, info: crate::task::WorkerInfo, tx: tokio::sync::mpsc::UnboundedSender<crate::task::MasterMessage>) {
        let id = info.id;
        self.workers.write().await.insert(id, info);
        self.worker_channels.write().await.insert(id, tx);
        self.num_workers.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.broadcast_workers().await;
    }

    pub async fn remove_worker(&self, id: Uuid) {
        let hostname = {
            let workers = self.workers.read().await;
            workers.get(&id).map(|w| w.hostname.clone())
        };

        if let Some(host) = hostname {
            {
                self.workers.write().await.remove(&id);
                self.worker_channels.write().await.remove(&id);
                self.num_workers.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                
                // Clean up task mappings for this worker
                let mut task_to_worker = self.task_to_worker.write().await;
                task_to_worker.retain(|_, v| v != &host);
            }
            
            self.broadcast_workers().await;
            self.broadcast_status().await;
        }
    }

    pub async fn send_to_random_worker(&self, msg: crate::task::MasterMessage) -> Result<(), String> {
        let channels = self.worker_channels.read().await;
        if channels.is_empty() {
            return Err("No workers available".to_string());
        }

        use rand::seq::IteratorRandom;
        let mut rng = rand::rng();
        if let Some((_, tx)) = channels.iter().choose(&mut rng) {
            tx.send(msg).map_err(|e| format!("Failed to send to worker: {}", e))?;
            Ok(())
        } else {
            Err("Failed to select random worker".to_string())
        }
    }

    pub async fn get_workers(&self) -> Vec<crate::task::WorkerInfo> {
        self.workers.read().await.values().cloned().collect()
    }

    pub async fn get_worker_hostname(&self, id: Uuid) -> Option<String> {
        self.workers.read().await.get(&id).map(|w| w.hostname.clone())
    }

    pub async fn update_worker_metrics(&self, worker_id: Uuid, metrics: crate::task::SystemMetrics) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.get_mut(&worker_id) {
            worker.metrics = Some(metrics);
            drop(workers); // Release lock before broadcast
            self.broadcast_workers().await;
        }
    }

    pub async fn update_worker_task(&self, worker_id: Uuid, task_id: Uuid, task_name: String, started: bool) {
        let mut changed = false;
        {
            let mut workers = self.workers.write().await;
            if let Some(worker) = workers.get_mut(&worker_id) {
                if started {
                    if !worker.running_tasks.contains(&task_name) {
                        worker.running_tasks.push(task_name);
                    }
                    self.task_to_worker.write().await.insert(task_id, worker.hostname.clone());
                    
                    // Update last_host in the task itself
                    if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
                        task.last_host = Some(worker.hostname.clone());
                    }
                } else {
                    worker.running_tasks.retain(|t| t != &task_name);
                    self.task_to_worker.write().await.remove(&task_id);
                }
                changed = true;
            }
        }
        
        if changed {
            self.broadcast_workers().await;
            self.broadcast_status().await;
        }
    }

    pub fn track_worker(self: Arc<Self>, worker_id: Uuid) -> WorkerRegistration {
        WorkerRegistration { scheduler: self, worker_id }
    }
}

pub struct WorkerRegistration {
    scheduler: Arc<Scheduler>,
    worker_id: Uuid,
}

impl Drop for WorkerRegistration {
    fn drop(&mut self) {
        let scheduler = self.scheduler.clone();
        let id = self.worker_id;
        tokio::spawn(async move {
            scheduler.remove_worker(id).await;
        });
    }
}

impl Scheduler {
    pub async fn add_native_handler(&self, name: &str, handler: TaskHandler) {
        self.native_handlers.write().await.insert(name.to_string(), handler);
    }

    pub async fn reload_from_file(self: &Arc<Self>, path: &str) -> Result<(), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        *self.config_toml.write().await = Some(content.clone());
        
        let config = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;
        self.sync_with_config(&config).await
    }

    pub fn subscribe_logs(&self) -> tokio::sync::broadcast::Receiver<LogMessage> {
        self.log_sender.subscribe()
    }

    pub fn subscribe_status(&self) -> tokio::sync::broadcast::Receiver<Vec<crate::task::TaskStatus>> {
        self.status_sender.subscribe()
    }

    pub fn subscribe_webhooks(&self) -> tokio::sync::broadcast::Receiver<crate::db::WebhookLog> {
        self.webhook_sender.subscribe()
    }

    pub fn subscribe_workers(&self) -> tokio::sync::broadcast::Receiver<Vec<crate::task::WorkerInfo>> {
        self.worker_sender.subscribe()
    }

    pub async fn broadcast_workers(&self) {
        let workers = self.get_workers().await;
        let _ = self.worker_sender.send(workers);
    }

    pub fn broadcast_webhook(&self, webhook: crate::db::WebhookLog) {
        let _ = self.webhook_sender.send(webhook);
    }

    pub fn subscribe_worker_messages(&self) -> tokio::sync::broadcast::Receiver<crate::task::MasterMessage> {
        self.worker_tx.subscribe()
    }

    pub fn send_worker_message(&self, msg: crate::task::MasterMessage) {
        let _ = self.worker_tx.send(msg);
    }

    pub fn send_log(&self, msg: LogMessage) {
        let _ = self.log_sender.send(msg);
    }

    pub async fn get_bootstrap_info(&self) -> crate::task::MasterMessage {
        crate::task::MasterMessage::Bootstrap(crate::task::BootstrapInfo {
            config_toml: self.config_toml.read().await.clone().unwrap_or_default(),
            wasm_paths: self.wasm_cache.read().await.keys().cloned().collect(),
        })
    }

    pub async fn get_wasm_binary(&self, path: &str) -> Option<Arc<Vec<u8>>> {
        self.wasm_cache.read().await.get(path).cloned()
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
        let task_to_worker = self.task_to_worker.read().await;
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
                last_log_id: t.last_log_id,
                host: task_to_worker.get(&t.id).cloned(),
                last_host: t.last_host.clone(),
            })
            .collect()
    }

    // Sync tasks with configuration
    pub async fn sync_with_config(
        self: &Arc<Self>,
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

        let mut tasks_to_register = Vec::new();

        // 2. Add or Update tasks from config
        for task_cfg in &config.tasks {
            config_names.insert(task_cfg.name.clone());
            
            let mut should_update = true;
            let mut existing_state = None;

            if let Some(id) = existing_tasks.get(&task_cfg.name) {
                let tasks = self.tasks.read().await;
                if let Some(existing) = tasks.get(id) {
                    // Check if config actually changed
                    if existing.config_equals(&task_cfg.cron, &task_cfg.timezone, &task_cfg.task_type, task_cfg.payload.as_deref(), task_cfg.args.as_deref(), task_cfg.env.as_ref(), task_cfg.sha256.as_deref()) {
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
                        if native_handlers.contains_key(&task_cfg.name) {
                            tasks_to_register.push((*id, task_cfg.name.clone(), task_cfg.task_type.clone(), None, None, None, None));
                        }
                    }
                }
                continue;
            }

            let task_id = existing_tasks.get(&task_cfg.name).cloned().unwrap_or_else(Uuid::new_v4);

            match task_cfg.task_type.as_str() {
                "native" => {
                    if native_handlers.contains_key(&task_cfg.name) {
                        let mut task = ScheduledTask::new(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone)?;
                        task.id = task_id;
                        
                        // Preserve state if updating
                        if let Some((last_run, last_dur, last_failed, enabled)) = existing_state {
                            task.last_run = last_run;
                            task.last_duration = last_dur;
                            task.last_failed = last_failed;
                            task.enabled = enabled;
                        }

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "native", None, None, None, None, task.enabled).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        tasks_to_register.push((task_id, task_cfg.name.clone(), task_cfg.task_type.clone(), None, None, None, None));
                        println!("Updated native task '{}'", task_cfg.name);
                    } else {
                        println!("Warning: No native handler found for task '{}'", task_cfg.name);
                    }
                }
                "wasm" => {
                    if let Some(payload) = &task_cfg.payload {
                        let mut task = ScheduledTask::new_wasm(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone, payload, task_cfg.args.clone(), task_cfg.env.clone(), task_cfg.sha256.clone())?;
                        task.id = task_id;

                        // Preserve state if updating
                        if let Some((last_run, last_dur, last_failed, enabled)) = existing_state {
                            task.last_run = last_run;
                            task.last_duration = last_dur;
                            task.last_failed = last_failed;
                            task.enabled = enabled;
                        }

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "wasm", Some(payload), task_cfg.args.clone(), task_cfg.env.clone(), task_cfg.sha256.as_deref(), task.enabled).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        tasks_to_register.push((task_id, task_cfg.name.clone(), task_cfg.task_type.clone(), Some(payload.clone()), task_cfg.args.clone(), task_cfg.env.clone(), task_cfg.sha256.clone()));
                        println!("Updated WASM task '{}'", task_cfg.name);
                    }
                }
                _ => println!("Warning: Unknown task type '{}'", task_cfg.task_type),
            }
        }

        // Now register handlers without holding the tasks lock
        for (id, name, task_type, payload, args, env, sha256) in tasks_to_register {
            if task_type == "wasm" {
                if let Some(path) = payload {
                    self.clone().register_wasm_handler(id, path, name, args, env, sha256).await?;
                }
            } else if task_type == "native" {
                let native_handlers = self.native_handlers.read().await;
                if let Some(handler) = native_handlers.get(&name) {
                    let handler = Arc::clone(handler);
                    self.register_handler(id, move |log_id, task_id, db, log_sender| handler(log_id, task_id, db, log_sender)).await?;
                }
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

        self.broadcast_status().await;
        Ok(())
    }

    pub async fn sync_db_only(&self, config: &crate::config::AppConfig) -> Result<(), String> {
        let db_tasks = self.db.get_tasks().await
            .map_err(|e| format!("Failed to get tasks from DB: {}", e))?;
        
        let mut existing_info = HashMap::new();
        for (id, name, _, _, _, _, _, _, _, enabled) in db_tasks {
            existing_info.insert(name, (id, enabled));
        }

        let mut config_names = std::collections::HashSet::new();

        for task_cfg in &config.tasks {
            config_names.insert(task_cfg.name.clone());
            let (task_id, enabled) = existing_info.get(&task_cfg.name)
                .cloned()
                .unwrap_or_else(|| (Uuid::new_v4(), true));
            
            self.db.save_task(
                task_id, 
                &task_cfg.name, 
                &task_cfg.cron, 
                &task_cfg.timezone, 
                &task_cfg.task_type, 
                task_cfg.payload.as_deref(), 
                task_cfg.args.clone(), 
                task_cfg.env.clone(), 
                task_cfg.sha256.as_deref(), 
                enabled
            ).await.map_err(|e| format!("Failed to save task to DB: {}", e))?;
        }

        // Remove tasks from DB not in config
        for (name, (id, _)) in existing_info {
            if !config_names.contains(&name) {
                self.db.remove_task(id).await.map_err(|e| format!("Failed to remove task from DB: {}", e))?;
            }
        }

        Ok(())
    }

    // Load tasks from the database
    pub async fn load_tasks(self: &Arc<Self>) -> Result<Vec<(Uuid, String, String)>, String> {
        let db_tasks = self.db.get_tasks().await
            .map_err(|e| format!("Failed to get tasks from DB: {}", e))?;

        // Fetch latest run info from logs
        let latest_logs = self.db.get_latest_task_logs().await
            .unwrap_or_default();

        let mut tasks_to_register = Vec::new();
        let mut loaded = Vec::new();

        {
            let mut tasks = self.tasks.write().await;
            let native_handlers = self.native_handlers.read().await;

            for (id, name, cron_expr, timezone_str, task_type, payload, args, env, sha256, enabled) in db_tasks {
                match ScheduledTask::from_db(id, name.clone(), cron_expr, timezone_str, task_type.clone(), payload.clone(), args.clone(), env.clone(), sha256.clone(), enabled) {
                    Ok(mut task) => {
                        // Apply historical state
                        if let Some((log_id, run_at_str, duration_ms)) = latest_logs.get(&id) {
                            if let Ok(run_at) = DateTime::parse_from_rfc3339(run_at_str) {
                                task.last_run = Some(run_at.with_timezone(&Utc));
                            }
                            task.last_duration = duration_ms.map(|d| d as u64);
                            task.last_log_id = Some(*log_id);
                        }

                        tasks.insert(id, task);
                        loaded.push((id, name.clone(), task_type.clone()));

                        // Collect info for registration after releasing lock
                        if task_type == "wasm" {
                            if let Some(path) = payload {
                                tasks_to_register.push((id, name.clone(), task_type.clone(), Some(path), args, env, sha256));
                            }
                        } else if task_type == "native" {
                            if native_handlers.contains_key(&name) {
                                tasks_to_register.push((id, name.clone(), task_type.clone(), None, None, None, None));
                            }
                        }
                    }
                    Err(e) => println!("Warning: Failed to load task '{}': {}", name, e),
                }
            }
            println!("Loaded {} tasks from database", tasks.len());
        }

        // Now register handlers without holding the tasks lock
        for (id, name, task_type, payload, args, env, sha256) in tasks_to_register {
            if task_type == "wasm" {
                if let Some(path) = payload {
                    self.clone().register_wasm_handler(id, path, name, args, env, sha256).await?;
                }
            } else if task_type == "native" {
                let native_handlers = self.native_handlers.read().await;
                if let Some(handler) = native_handlers.get(&name) {
                    let handler = Arc::clone(handler);
                    self.register_handler(id, move |log_id, task_id, db, log_sender| handler(log_id, task_id, db, log_sender)).await?;
                }
            }
        }

        Ok(loaded)
    }
    async fn register_wasm_handler(self: Arc<Self>, task_id: Uuid, wasm_path: String, task_name: String, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, expected_sha256: Option<String>) -> Result<(), String> {
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

        // Cache the binary for bootstrapping workers
        let binary = Arc::new(binary);
        self.wasm_cache.write().await.insert(wasm_path.clone(), Arc::clone(&binary));

        // Verify SHA256
        if let Some(expected) = &expected_sha256 {
            let mut hasher = Sha256::new();
            hasher.update(&*binary);
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
        let self_arc = Arc::clone(&self);
        let sha256_clone = expected_sha256.clone();

        let handler = move |log_id: Uuid, task_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<LogMessage>| {
            let engine = engine.clone();
            let name = task_name.clone();
            let args = args.clone();
            let env = env.clone();
            let binary = binary.clone();
            let err_path = path_for_error.clone();
            let self_clone = Arc::clone(&self_arc);
            let expected_sha256 = sha256_clone.clone();

            Box::pin(async move {
                let resolved_args = wasm_handlers::resolve_args(args).await;
                
                if self_clone.num_workers() > 0 {
                    println!("[{}] Delegating WASM task to a random worker", name);
                    let req = crate::task::RunRequest {
                        wasm_path: err_path,
                        expected_sha256,
                        task_name: name,
                        args: resolved_args,
                        env,
                        log_id,
                        task_id,
                    };
                    
                    if let Err(e) = self_clone.send_to_random_worker(crate::task::MasterMessage::RunTask(req)).await {
                        let err_msg = format!("Failed to delegate task: {}", e);
                        eprintln!("{}", err_msg);
                        return Err(err_msg);
                    }
                    Ok(())
                } else {
                    let sink = Arc::new(wasm_handlers::DbLogSink {
                        db: db.clone(),
                        sender: log_sender,
                        hostname: "master".to_string(),
                    });
                    if let Err(e) = wasm_handlers::run_wasm_binary(&engine, &binary, &err_path, &name, sink, resolved_args, env, log_id, task_id).await {
                        let err_msg = format!("Error executing WASM task: {}", e);
                        eprintln!("{}", err_msg);
                        Err(err_msg)
                    } else {
                        Ok(())
                    }
                }
            }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        };

        self.handlers.write().await.insert(task_id, Arc::new(handler));
        Ok(())
    }

    // Register a handler for a specific task ID
    pub async fn register_handler<F, Fut>(&self, task_id: Uuid, handler: F) -> Result<(), String>
    where
        F: Fn(Uuid, Uuid, Db, tokio::sync::broadcast::Sender<LogMessage>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let wrapped_handler = move |log_id, task_id, db, log_sender| {
            let h = Arc::clone(&handler);
            Box::pin(async move { h(log_id, task_id, db, log_sender).await }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
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
        F: Fn(Uuid, Uuid, Db, tokio::sync::broadcast::Sender<LogMessage>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let task = ScheduledTask::new(name, cron_expr, timezone)?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "native", None, None, None, None, true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.register_handler(task_id, handler).await?;

        println!("Registered native task '{}' [{}] with id {}", name, timezone, task_id);
        self.broadcast_status().await;
        Ok(task_id)
    }

    // Register a new WASM task
    pub async fn add_wasm_task(
        self: Arc<Self>,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        wasm_path: &str,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
        sha256: Option<String>,
    ) -> Result<Uuid, String> {
        let task = ScheduledTask::new_wasm(name, cron_expr, timezone, wasm_path, args.clone(), env.clone(), sha256.clone())?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "wasm", Some(wasm_path), args.clone(), env.clone(), sha256.as_deref(), true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.clone().register_wasm_handler(task_id, wasm_path.to_string(), name.to_string(), args, env, sha256).await?;

        println!("Registered WASM task '{}' [{}] with id {}", name, timezone, task_id);
        self.broadcast_status().await;
        Ok(task_id)
    }

    // Remove a task from the scheduler
    pub async fn remove_task(&self, task_id: Uuid) -> bool {
        let _ = self.db.remove_task(task_id).await;
        let task_removed = {
            let mut tasks = self.tasks.write().await;
            self.handlers.write().await.remove(&task_id);
            tasks.remove(&task_id).is_some()
        };
        
        if task_removed {
            self.broadcast_status().await;
        }
        task_removed
    }

    // Enable or disable a task without removing it
    pub async fn set_task_enabled(&self, task_id: Uuid, enabled: bool) {
        let _ = self.db.update_task_enabled(task_id, enabled).await;
        {
            if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
                task.enabled = enabled;
            }
        }
        self.broadcast_status().await;
    }

    pub async fn run_task_immediately(self: Arc<Self>, task_id: Uuid) -> Result<(), String> {
        let (task_name, is_enabled) = {
            let tasks = self.tasks.read().await;
            let task = tasks.get(&task_id).ok_or_else(|| "Task not found".to_string())?;
            (task.name.clone(), task.enabled)
        };

        if !is_enabled {
            return Err("Cannot run a disabled task".to_string());
        }

        let handlers = self.handlers.read().await;
        let handler = handlers.get(&task_id).ok_or_else(|| "Handler not found".to_string())?;
        let handler = Arc::clone(handler);

        let db = self.db.clone();
        let tasks_ref = Arc::clone(&self.tasks);
        let log_sender = self.log_sender.clone();
        let self_arc = Arc::clone(&self);

        tokio::spawn(async move {
            if let Ok(log_id) = db.log_execution_start(task_id).await {
                let start = std::time::Instant::now();
                let start_msg = format!("[{}] Manually starting task...", task_name);
                println!("{}", start_msg);
                let _ = db.save_log_line(log_id, &task_name, Some("master"), "Manually starting task...").await;
                let _ = log_sender.send(LogMessage { 
                            task_id, 
                            log_id: Some(log_id), 
                            prefix: Some(task_name.clone()), 
                            hostname: Some("master".to_string()),
                            text: start_msg 
                        });

                let result = handler(log_id, task_id, db.clone(), log_sender.clone()).await;
                let is_failed = result.is_err();

                if let Err(e) = &result {
                    let err_msg = format!("[{}] Task failed: {}", task_name, e);
                    println!("{}", err_msg);
                    let _ = db.save_log_line(log_id, &task_name, Some("master"), &format!("Task failed: {}", e)).await;
                    let _ = log_sender.send(LogMessage { 
                        task_id, 
                        log_id: Some(log_id), 
                        prefix: Some(task_name.clone()), 
                        hostname: Some("master".to_string()),
                        text: err_msg 
                    });
                }

                let duration_ms = start.elapsed().as_millis() as u64;
                let finish_msg = format!("[{}] Task finished in {}ms", task_name, duration_ms);
                println!("{}", finish_msg);
                let _ = db.save_log_line(log_id, &task_name, Some("master"), &format!("Task finished in {}ms", duration_ms)).await;
                let _ = log_sender.send(LogMessage { 
                    task_id, 
                    log_id: Some(log_id), 
                    prefix: Some(task_name.clone()), 
                    hostname: Some("master".to_string()),
                    text: finish_msg 
                });                let _ = db.log_execution_finish(log_id, duration_ms).await;
                
                {
                    let mut tasks = tasks_ref.write().await;
                    if let Some(task) = tasks.get_mut(&task_id) {
                        task.last_duration = Some(duration_ms);
                        task.last_failed = is_failed;
                        task.last_log_id = Some(log_id);
                        task.last_host = Some("master".to_string());
                    }
                }
                self_arc.broadcast_status().await;
            }
        });

        Ok(())
    }

    pub async fn broadcast_status(&self) {
        let status = self.get_tasks_status().await;
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

                if task.should_run() {
                    println!("Task '{}' triggered (last run was: {:?})", task.name, task.last_run);
                    task.last_run = Some(Utc::now());
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
                        let _ = db.save_log_line(log_id, &name, Some("master"), "Starting task...").await;
                        let _ = log_sender.send(LogMessage { 
                            task_id, 
                            log_id: Some(log_id), 
                            prefix: Some(name.clone()), 
                            hostname: Some("master".to_string()),
                            text: start_msg 
                        });
                        
                        let result = handler(log_id, task_id, db.clone(), log_sender.clone()).await;
                        let is_failed = result.is_err();
                        
                        if let Err(e) = &result {
                            let err_msg = format!("[{}] Task failed: {}", name, e);
                            println!("{}", err_msg);
                            let _ = db.save_log_line(log_id, &name, Some("master"), &format!("Task failed: {}", e)).await;
                            let _ = log_sender.send(LogMessage { 
                                task_id, 
                                log_id: Some(log_id), 
                                prefix: Some(name.clone()), 
                                hostname: Some("master".to_string()),
                                text: err_msg 
                            });
                        }
                        
                        let duration_ms = start.elapsed().as_millis() as u64;
                        let finish_msg = format!("[{}] Task finished in {}ms", name, duration_ms);
                        println!("{}", finish_msg);
                        let _ = db.save_log_line(log_id, &name, Some("master"), &format!("Task finished in {}ms", duration_ms)).await;
                        let _ = log_sender.send(LogMessage { 
                            task_id, 
                            log_id: Some(log_id), 
                            prefix: Some(name.clone()), 
                            hostname: Some("master".to_string()),
                            text: finish_msg 
                        });
                        
                        let _ = db.log_execution_finish(log_id, duration_ms).await;
                        
                        // Update in-memory duration and status
                        {
                            let mut tasks = tasks_ref.write().await;
                            if let Some(task) = tasks.get_mut(&task_id) {
                                task.last_duration = Some(duration_ms);
                                task.last_failed = is_failed;
                                task.last_log_id = Some(log_id);
                                if task.task_type == "native" {
                                    task.last_host = Some("master".to_string());
                                }
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
