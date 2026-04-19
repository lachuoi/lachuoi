use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use uuid::Uuid;

use crate::task::ScheduledTask;

// Type alias for task handlers - functions that execute when a task runs
type TaskHandler = Arc<dyn Fn() + Send + Sync>;

pub struct Scheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, TaskHandler>>>,
    running: Arc<RwLock<bool>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    // Register a new task with its handler function
    pub async fn add_task<F>(
        &self,
        name: &str,
        cron_expr: &str,
        handler: F,
    ) -> Result<Uuid, String>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let task = ScheduledTask::new(name, cron_expr)
            .map_err(|e| format!("Invalid cron expression: {}", e))?;

        let task_id = task.id;

        self.tasks.write().await.insert(task_id, task);
        self.handlers
            .write()
            .await
            .insert(task_id, Arc::new(handler));

        println!("Registered task '{}' with id {}", name, task_id);

        Ok(task_id)
    }

    // Remove a task from the scheduler
    pub async fn remove_task(&self, task_id: Uuid) -> bool {
        let task_removed = self.tasks.write().await.remove(&task_id).is_some();
        self.handlers.write().await.remove(&task_id);
        task_removed
    }

    // Enable or disable a task without removing it
    pub async fn set_task_enabled(&self, task_id: Uuid, enabled: bool) {
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
        let now = Utc::now();
        let mut tasks_to_run = Vec::new();

        // Collect tasks that need to run
        {
            let mut tasks = self.tasks.write().await;
            for (id, task) in tasks.iter_mut() {
                if !task.enabled {
                    continue;
                }

                // Check if we should run based on the schedule
                if let Some(last_run) = task.last_run {
                    // Skip if we already ran this minute
                    if now.signed_duration_since(last_run).num_seconds() < 60 {
                        continue;
                    }
                }

                // Check if current time matches the cron schedule
                if task.should_run() {
                    task.last_run = Some(now);
                    tasks_to_run.push(*id);
                }
            }
        }

        // Execute handlers for due tasks
        let handlers = self.handlers.read().await;
        for task_id in tasks_to_run {
            if let Some(handler) = handlers.get(&task_id) {
                let handler = Arc::clone(handler);

                // Spawn task execution in a separate thread to avoid blocking
                tokio::spawn(async move {
                    handler();
                });
            }
        }
    }
}
