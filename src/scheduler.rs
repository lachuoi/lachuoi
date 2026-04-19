use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;
use wasmtime::*;
use wasmtime_wasi::preview1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;
use chrono_tz::Tz;

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
        let module = Module::from_file(engine, path)?;
        
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

    // Start the scheduler - runs until stop() is called
    pub async fn start(&self) {
        *self.running.write().await = true;

        // Check for tasks to run every second
        let mut ticker = interval(Duration::from_secs(1));

        println!("Scheduler started");

        while *self.running.read().await {
            ticker.tick().await;
            self.tick().await;
        }

        println!("Scheduler stopped");
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

                tokio::spawn(async move {
                    let _ = db.log_execution(task_id, "started").await;
                    handler();
                    let _ = db.log_execution(task_id, "completed").await;
                });
            }
        }
    }
}
