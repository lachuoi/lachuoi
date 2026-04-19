use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;
use wasmtime::*;
use wasmtime::component::{Component, Linker as ComponentLinker};
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::{WasiCtx, WasiView, WasiCtxBuilder, ResourceTable};
use wasmtime_wasi::bindings::Command;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView, add_only_http_to_linker_async as add_http_to_linker};
use chrono_tz::Tz;

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

// Type alias for task handlers - functions that execute when a task runs
type TaskHandler = Arc<dyn Fn() + Send + Sync>;

pub struct Scheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, TaskHandler>>>,
    running: Arc<RwLock<bool>>,
    db: Db,
    wasm_engine: Engine,
    plugins_dir: std::path::PathBuf,
}

impl Scheduler {
    pub fn new(db: Db) -> Self {
        let mut config = Config::new();
        config.async_support(true);
        config.wasm_component_model(true);
        let wasm_engine = Engine::new(&config).expect("Failed to create Wasmtime engine");

        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            db,
            wasm_engine,
            plugins_dir: std::path::PathBuf::from("plugins"),
        }
    }

    pub fn set_plugins_dir(&mut self, path: &str) {
        self.plugins_dir = std::path::PathBuf::from(path);
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
                enabled: t.enabled,
            })
            .collect()
    }

    // Sync tasks with configuration
    pub async fn sync_with_config(
        &self,
        config: &crate::config::AppConfig,
        native_handlers: &HashMap<String, Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<(), String> {
        // 1. Map existing tasks by name
        let existing_by_name = {
            let tasks = self.tasks.read().await;
            tasks.values()
                .map(|t| (t.name.clone(), (t.id, t.task_type.clone())))
                .collect::<HashMap<String, (Uuid, String)>>()
        };

        let mut config_names = std::collections::HashSet::new();

        // 2. Add or Update tasks from config
        for task_cfg in &config.tasks {
            config_names.insert(task_cfg.name.clone());
            
            let task_id = if let Some((id, _)) = existing_by_name.get(&task_cfg.name) {
                *id
            } else {
                Uuid::new_v4()
            };

            match task_cfg.task_type.as_str() {
                "native" => {
                    if let Some(handler) = native_handlers.get(&task_cfg.name) {
                        let handler = Arc::clone(handler);
                        let mut task = ScheduledTask::new(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone)?;
                        task.id = task_id; // Preserve ID if updating

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "native", None, true).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        self.register_handler(task_id, move || handler()).await?;
                    } else {
                        println!("Warning: No native handler found for task '{}'", task_cfg.name);
                    }
                }
                "wasm" => {
                    if let Some(payload) = &task_cfg.payload {
                        let mut task = ScheduledTask::new_wasm(&task_cfg.name, &task_cfg.cron, &task_cfg.timezone, payload)?;
                        task.id = task_id;

                        self.db.save_task(task_id, &task_cfg.name, &task_cfg.cron, &task_cfg.timezone, "wasm", Some(payload), true).await
                            .map_err(|e| format!("Failed to save task: {}", e))?;

                        self.tasks.write().await.insert(task_id, task);
                        self.register_wasm_handler(task_id, payload.to_string()).await?;
                    }
                }
                _ => println!("Warning: Unknown task type '{}'", task_cfg.task_type),
            }
        }

        // 3. Remove tasks that are in DB but NOT in config
        let to_remove: Vec<Uuid> = existing_by_name
            .iter()
            .filter(|(name, _)| !config_names.contains(*name))
            .map(|(_, (id, _))| *id)
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

        let mut tasks = self.tasks.write().await;
        let mut loaded = Vec::new();
        for (id, name, cron_expr, timezone_str, task_type, payload, enabled) in db_tasks {
            let timezone = timezone_str.parse::<Tz>()
                .map_err(|e| format!("Invalid timezone in DB: {}", e))?;
            
            match ScheduledTask::from_db(id, name.clone(), cron_expr, timezone, task_type.clone(), payload.clone(), enabled) {
                Ok(task) => {
                    tasks.insert(id, task);
                    loaded.push((id, name, task_type.clone()));
                    
                    // If it's a WASM task, automatically register its handler
                    if task_type == "wasm" {
                        if let Some(path) = payload {
                            self.register_wasm_handler(id, path).await?;
                        }
                    }
                }
                Err(e) => println!("Warning: Failed to load task '{}': {}", name, e),
            }
        }
        println!("Loaded {} tasks from database", tasks.len());
        Ok(loaded)
    }

    async fn register_wasm_handler(&self, task_id: Uuid, wasm_path: String) -> Result<(), String> {
        let engine = self.wasm_engine.clone();
        let full_path = if std::path::Path::new(&wasm_path).is_absolute() {
            std::path::PathBuf::from(&wasm_path)
        } else {
            self.plugins_dir.join(&wasm_path)
        };
        
        let path_for_error = wasm_path.clone();
        let handler = move || {
            let engine = engine.clone();
            let path = full_path.clone();
            let err_path = path_for_error.clone();
            
            tokio::spawn(async move {
                if let Err(e) = Self::run_wasm_file(&engine, path.to_str().unwrap_or(&err_path)).await {
                    eprintln!("Error executing WASM task: {}", e);
                }
            });
        };

        self.handlers.write().await.insert(task_id, Arc::new(handler));
        Ok(())
    }

    async fn run_wasm_file(engine: &Engine, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let binary = std::fs::read(path)?;
        
        // Check if it's a WASM component (starts with \0asm and version 0x0d)
        if binary.starts_with(&[0, 0x61, 0x73, 0x6d, 0x0d, 0, 1, 0]) {
            Self::run_wasm_component(engine, &binary).await
        } else {
            Self::run_wasm_module(engine, &binary).await
        }
    }

    async fn run_wasm_module(engine: &Engine, binary: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let module = Module::from_binary(engine, binary)?;
        
        struct MyState {
            wasi: WasiP1Ctx,
        }
        
        let mut linker = Linker::new(engine);
        preview1::add_to_linker_async(&mut linker, |state: &mut MyState| &mut state.wasi)?;
        
        let wasi = WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .build_p1();
            
        let mut store = Store::new(engine, MyState { wasi });
        
        let instance = linker.instantiate_async(&mut store, &module).await?;
        let func = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
        
        func.call_async(&mut store, ()).await?;
        
        Ok(())
    }

    async fn run_wasm_component(engine: &Engine, binary: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let component = Component::from_binary(engine, binary)?;
        
        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        add_http_to_linker(&mut linker)?;
        
        let wasi = WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .inherit_network()
            .allow_ip_name_lookup(true)
            .build();
            
        let http = WasiHttpCtx::new();
        let table = ResourceTable::new();
        
        let mut store = Store::new(engine, ComponentState { wasi, http, table });
        
        let command = Command::instantiate_async(&mut store, &component, &linker).await?;
        command.wasi_cli_run().call_run(&mut store).await?.map_err(|()| Box::<dyn std::error::Error + Send + Sync>::from("WASI run failed"))?;
        
        Ok(())
    }

    // Register a handler for a specific task ID
    pub async fn register_handler<F>(&self, task_id: Uuid, handler: F) -> Result<(), String>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.handlers
            .write()
            .await
            .insert(task_id, Arc::new(handler));
        Ok(())
    }

    // Register a new native task
    pub async fn add_task<F>(
        &self,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        handler: F,
    ) -> Result<Uuid, String>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let task = ScheduledTask::new(name, cron_expr, timezone)?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "native", None, true).await
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
    ) -> Result<Uuid, String> {
        let task = ScheduledTask::new_wasm(name, cron_expr, timezone, wasm_path)?;
        let task_id = task.id;

        self.db.save_task(task_id, name, cron_expr, timezone, "wasm", Some(wasm_path), true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.register_wasm_handler(task_id, wasm_path.to_string()).await?;

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

    // Start the scheduler - runs in the background
    pub async fn start(self: Arc<Self>) {
        *self.running.write().await = true;

        let scheduler = Arc::clone(&self);
        tokio::spawn(async move {
            // Check for tasks to run every second
            let mut ticker = interval(Duration::from_secs(1));

            println!("Scheduler background loop started");

            while *scheduler.running.read().await {
                ticker.tick().await;
                scheduler.tick().await;
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
    async fn tick(&self) {
        let mut tasks_to_run = Vec::new();

        // Collect tasks that need to run
        {
            let mut tasks = self.tasks.write().await;
            for (id, task) in tasks.iter_mut() {
                if !task.enabled {
                    continue;
                }

                let now_tz = Utc::now().with_timezone(&task.timezone);

                // Check if we should run based on the schedule
                if let Some(last_run) = task.last_run {
                    if now_tz.signed_duration_since(last_run).num_seconds() < 60 {
                        continue;
                    }
                }

                if task.should_run() {
                    task.last_run = Some(now_tz);
                    tasks_to_run.push(*id);
                }
            }
        }

        // Execute handlers for due tasks
        let handlers = self.handlers.read().await;
        for task_id in tasks_to_run {
            if let Some(handler) = handlers.get(&task_id) {
                let handler = Arc::clone(handler);
                let db = self.db.clone();
                let tasks_ref = Arc::clone(&self.tasks);

                tokio::spawn(async move {
                    if let Ok(log_id) = db.log_execution_start(task_id).await {
                        let start = std::time::Instant::now();
                        handler();
                        let duration = start.elapsed().as_millis();
                        let _ = db.log_execution_finish(log_id, duration).await;
                        
                        // Update in-memory duration
                        let mut tasks = tasks_ref.write().await;
                        if let Some(task) = tasks.get_mut(&task_id) {
                            task.last_duration = Some(duration);
                        }
                    }
                });
            }
        }
    }
}
