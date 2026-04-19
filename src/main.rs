use chrono::Utc;
use dotenvy::dotenv;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use task_scheduler::config::AppConfig;
use task_scheduler::db::Db;
use task_scheduler::scheduler::Scheduler;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let db_url = std::env::var("TURSO_DATABASE_URL")
        .unwrap_or_else(|_| "tasks.db".to_string());
    let auth_token = std::env::var("TURSO_AUTH_TOKEN").ok();

    let db = Db::new(&db_url, auth_token.as_deref())
        .await
        .expect("Failed to initialize database");
    let scheduler = Scheduler::new(db.clone());

    // 1. Register native handlers
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);
    let mut native_handlers: HashMap<String, Arc<dyn Fn() + Send + Sync>> = HashMap::new();

    native_handlers.insert("heartbeat".to_string(), Arc::new(move || {
        let count = counter_clone.fetch_add(1, Ordering::SeqCst);
        println!("[{}] Heartbeat #{}", Utc::now().format("%H:%M:%S"), count + 1);
    }));

    native_handlers.insert("hourly-report".to_string(), Arc::new(|| {
        println!("[{}] Generating hourly report...", Utc::now().format("%H:%M:%S"));
    }));

    native_handlers.insert("cache-cleanup".to_string(), Arc::new(|| {
        println!("[{}] Cleaning up cache...", Utc::now().format("%H:%M:%S"));
    }));

    // 2. Load current state from DB
    let _ = scheduler.load_tasks().await.expect("Failed to load tasks from DB");

    // 3. Always sync with cron.toml (Source of Truth)
    if std::path::Path::new("cron.toml").exists() {
        println!("Syncing with cron.toml...");
        let config = AppConfig::load("cron.toml").expect("Failed to load cron.toml");
        scheduler.sync_with_config(&config, native_handlers).await.expect("Failed to sync config");
    } else {
        println!("cron.toml not found, running with existing database tasks.");
    }

    // 4. Start the scheduler
    scheduler.start().await;
}
