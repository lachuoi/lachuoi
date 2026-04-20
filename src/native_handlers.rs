use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::pin::Pin;
use std::future::Future;
use uuid::Uuid;
use crate::db::Db;
use crate::scheduler::Scheduler;

/// Registers all native handlers to the provided scheduler
pub async fn register_all(scheduler: &Scheduler) {
    // 1. Heartbeat handler (stateful)
    let counter = Arc::new(AtomicU32::new(0));
    scheduler.add_native_handler("heartbeat", heartbeat_handler(counter)).await;

    // 2. Hourly report handler
    scheduler.add_native_handler("hourly-report", Arc::new(hourly_report_handler)).await;

    // 3. Cache cleanup handler
    scheduler.add_native_handler("cache-cleanup", Arc::new(cache_cleanup_handler)).await;
}

/// Heartbeat handler with incrementing counter
fn heartbeat_handler(counter: Arc<AtomicU32>) -> Arc<dyn Fn(Uuid, Db, tokio::sync::broadcast::Sender<String>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync> {
    Arc::new(move |log_id, db, log_sender| {
        let counter = Arc::clone(&counter);
        Box::pin(async move {
            let count = counter.fetch_add(1, Ordering::SeqCst);
            let msg = format!(
                "[heartbeat] Heartbeat #{}",
                count + 1
            );
            println!("{}", msg);
            let _ = db.save_log_line(log_id, &msg).await;
            let _ = log_sender.send(msg);
            Ok(())
        }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    })
}

/// Hourly report placeholder
fn hourly_report_handler(log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        let msg = "[hourly-report] Generating hourly report...".to_string();
        println!("{}", msg);
        let _ = db.save_log_line(log_id, &msg).await;
        let _ = log_sender.send(msg);
        Ok(())
    })
}

/// Cache cleanup placeholder
fn cache_cleanup_handler(log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        let msg = "[cache-cleanup] Cleaning up cache...".to_string();
        println!("{}", msg);
        let _ = db.save_log_line(log_id, &msg).await;
        let _ = log_sender.send(msg);
        Ok(())
    })
}
