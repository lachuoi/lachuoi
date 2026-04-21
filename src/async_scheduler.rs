use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use uuid::Uuid;

use crate::task::ScheduledTask;
// Type for async task handlers
type AsyncTaskHandler =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub struct AsyncScheduler {
    tasks: Arc<RwLock<HashMap<Uuid, ScheduledTask>>>,
    handlers: Arc<RwLock<HashMap<Uuid, AsyncTaskHandler>>>,
    running: Arc<RwLock<bool>>,
}

impl AsyncScheduler {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    // Register an async task handler
    pub async fn add_async_task<F, Fut>(
        &self,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        handler: F,
    ) -> Result<Uuid, String>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let task = ScheduledTask::new(name, cron_expr, timezone)?;

        let task_id = task.id;

        // Wrap the handler to return a pinned boxed future
        let wrapped_handler: AsyncTaskHandler = Arc::new(move || {
            Box::pin(handler()) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        self.tasks.write().await.insert(task_id, task);
        self.handlers.write().await.insert(task_id, wrapped_handler);

        Ok(task_id)
    }

    // Execute async handlers
    async fn run_task(&self, task_id: Uuid) {
        let handler = {
            let handlers = self.handlers.read().await;
            handlers.get(&task_id).cloned()
        };

        if let Some(handler) = handler {
            tokio::spawn(async move {
                handler().await;
            });
        }
    }
}
