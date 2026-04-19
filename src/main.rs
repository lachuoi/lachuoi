use chrono::Utc;
use dotenvy::dotenv;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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

    // Load tasks from database
    let loaded_tasks =
        scheduler.load_tasks().await.expect("Failed to load tasks");

    // Counter to track executions
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    // Define handlers once
    let heartbeat_handler = move || {
        let count = counter_clone.fetch_add(1, Ordering::SeqCst);
        println!(
            "[{}] Heartbeat #{}",
            Utc::now().format("%H:%M:%S"),
            count + 1
        );
    };

    let report_handler = || {
        println!(
            "[{}] Generating hourly report...",
            Utc::now().format("%H:%M:%S")
        );
    };

    let cleanup_handler = || {
        println!("[{}] Cleaning up cache...", Utc::now().format("%H:%M:%S"));
    };

    if loaded_tasks.is_empty() {
        // Initial setup: add tasks and register handlers with timezones
        scheduler
            .add_task("heartbeat", "0 * * * * *", "UTC", heartbeat_handler)
            .await
            .unwrap();
        scheduler
            .add_task("hourly-report", "0 0 * * * *", "UTC", report_handler)
            .await
            .unwrap();
        scheduler
            .add_task(
                "cache-cleanup",
                "0 */5 * * * *",
                "Asia/Seoul",
                cleanup_handler,
            )
            .await
            .unwrap();

        // Add a WASM task (assuming hello.wasm exists)
        let _ = scheduler
            .add_wasm_task("wasm-hello", "*/2 * * * * *", "UTC", "hello.wasm")
            .await
            .map_err(|e| println!("WASM task skip (local only): {}", e));
    } else {
        // Re-register handlers for loaded tasks
        println!("Database not empty, re-registering handlers.");
        for (id, name, task_type) in loaded_tasks {
            if task_type == "native" {
                match name.as_str() {
                    "heartbeat" => {
                        scheduler
                            .register_handler(id, heartbeat_handler.clone())
                            .await
                            .unwrap();
                    }
                    "hourly-report" => {
                        scheduler
                            .register_handler(id, report_handler)
                            .await
                            .unwrap();
                    }
                    "cache-cleanup" => {
                        scheduler
                            .register_handler(id, cleanup_handler)
                            .await
                            .unwrap();
                    }
                    _ => {
                        println!(
                            "Warning: No handler defined for native task '{}'",
                            name
                        )
                    }
                }
            }
            // WASM handlers are automatically re-registered in scheduler.load_tasks()
        }
    }

    // Start the scheduler
    scheduler.start().await;
}
