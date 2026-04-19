use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;

use crate::task::ScheduledTask;
use crate::db::Db;

// Type alias for task handlers - functions that execute when a task runs
type TaskHandler = Arc<dyn Fn() + Send + Sync>;

pub struct Scheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, TaskHandler>>>,
    running: Arc<RwLock<bool>>,
    db: Db,
}

impl Scheduler {
    pub fn new(db: Db) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            db,
        }
    }

    // Load tasks from the database
    pub async fn load_tasks(&self) -> Result<Vec<(Uuid, String)>, String> {
        let db_tasks = self.db.get_tasks().await
            .map_err(|e| format!("Failed to load tasks from DB: {}", e))?;

        let mut tasks = self.tasks.write().await;
        let mut loaded = Vec::new();
        for (id, name, cron_expr, timezone, enabled) in db_tasks {
            match ScheduledTask::new(&name, &cron_expr, &timezone) {
                Ok(mut task) => {
                    task.id = id;
                    task.enabled = enabled;
                    tasks.insert(id, task);
                    loaded.push((id, name));
                }
                Err(e) => println!("Warning: Failed to parse task '{}': {}", name, e),
            }
        }
        println!("Loaded {} tasks from database", tasks.len());
        Ok(loaded)
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

    // Register a new task with its handler function and timezone
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

        // Save to database
        self.db.save_task(task_id, name, cron_expr, timezone, true).await
            .map_err(|e| format!("Failed to save task to DB: {}", e))?;

        self.tasks.write().await.insert(task_id, task);
        self.register_handler(task_id, handler).await?;

        println!("Registered task '{}' [{}] with id {}", name, timezone, task_id);

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
                    // Skip if we already ran this minute (comparing in same timezone)
                    if now_tz.signed_duration_since(last_run).num_seconds() < 60 {
                        continue;
                    }
                }

                // Check if current time matches the cron schedule
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

                // Spawn task execution in a separate thread to avoid blocking
                tokio::spawn(async move {
                    let _ = db.log_execution(task_id, "started").await;
                    handler();
                    let _ = db.log_execution(task_id, "completed").await;
                });
            }
        }
    }
}
