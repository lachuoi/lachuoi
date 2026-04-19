use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::pin::Pin;
use task_scheduler::config::AppConfig;
use task_scheduler::db::Db;
use task_scheduler::scheduler::Scheduler;
use task_scheduler::web::WebServer;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    // 1. Initialize Database
    let db_url = std::env::var("TURSO_DATABASE_URL")
        .unwrap_or_else(|_| "tasks.db".to_string());
    
    let auth_token = if db_url.starts_with("libsql://") {
        std::env::var("TURSO_AUTH_TOKEN").ok()
    } else {
        None
    };

    let db = Db::new(&db_url, auth_token.as_deref())
        .await
        .expect("Failed to initialize database");

    // 2. Initialize Scheduler
    let scheduler = Arc::new(Scheduler::new(db.clone()));

    // 3. Define and register native handlers
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);
    let mut native_handlers: HashMap<String, Arc<dyn Fn(Uuid, Db, tokio::sync::broadcast::Sender<String>) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> + Send + Sync>> =
        HashMap::new();

    native_handlers.insert(
        "heartbeat".to_string(),
        Arc::new(move |log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>| {
            let counter_clone = Arc::clone(&counter_clone);
            Box::pin(async move {
                let count = counter_clone.fetch_add(1, Ordering::SeqCst);
                let msg = format!(
                    "[{}] Heartbeat #{}",
                    Utc::now().format("%H:%M:%S"),
                    count + 1
                );
                println!("{}", msg);
                let _ = db.save_log_line(log_id, &msg).await;
                let _ = log_sender.send(msg);
                Ok(())
            })
        }),
    );

    native_handlers.insert(
        "hourly-report".to_string(),
        Arc::new(|log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>| {
            Box::pin(async move {
                let msg = format!(
                    "[{}] Generating hourly report...",
                    Utc::now().format("%H:%M:%S")
                );
                println!("{}", msg);
                let _ = db.save_log_line(log_id, &msg).await;
                let _ = log_sender.send(msg);
                Ok(())
            })
        }),
    );

    native_handlers.insert(
        "cache-cleanup".to_string(),
        Arc::new(|log_id: Uuid, db: Db, log_sender: tokio::sync::broadcast::Sender<String>| {
            Box::pin(async move {
                let msg = format!(
                    "[{}] Cleaning up cache...",
                    Utc::now().format("%H:%M:%S")
                );
                println!("{}", msg);
                let _ = db.save_log_line(log_id, &msg).await;
                let _ = log_sender.send(msg);
                Ok(())
            })
        }),
    );

    // 4. Load state and sync with configuration
    let _ = scheduler
        .load_tasks()
        .await
        .expect("Failed to load tasks from DB");

    if std::path::Path::new("cron.toml").exists() {
        println!("Syncing with cron.toml...");
        let config =
            AppConfig::load("cron.toml").expect("Failed to load cron.toml");
        scheduler
            .sync_with_config(&config, &native_handlers)
            .await
            .expect("Failed to sync config");
    }

    // 5. Start the scheduler background loop
    scheduler.clone().start().await;

    // 6. Start the web server (blocking)
    let web_server = WebServer::new(Arc::clone(&scheduler), 9130);
    if let Err(e) = web_server.run().await {
        eprintln!("Web server error: {}", e);
    }
}
