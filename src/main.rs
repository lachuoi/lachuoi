use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::pin::Pin;
use task_scheduler::db::Db;
use task_scheduler::scheduler::Scheduler;
use task_scheduler::web::WebServer;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let pid_file = ".scheduler.pid";

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "reload" {
        use tokio::process::Command;
        
        let pid = match std::fs::read_to_string(pid_file) {
            Ok(content) => content.trim().to_string(),
            Err(_) => {
                eprintln!("Error: PID file '{}' not found. Is the server running?", pid_file);
                return;
            }
        };

        let output = Command::new("kill")
            .arg("-HUP")
            .arg(&pid)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => println!("Reload signal sent successfully to PID {}.", pid),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                eprintln!("Error sending reload signal to PID {}: {}", pid, err);
            }
            Err(e) => eprintln!("Failed to execute kill: {}", e),
        }
        return;
    }

    // Write PID file for the server process
    if let Err(e) = std::fs::write(pid_file, std::process::id().to_string()) {
        eprintln!("Warning: Could not write PID file: {}", e);
    }

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
    
    let heartbeat_handler = {
        let counter_clone = Arc::clone(&counter);
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
            }) as Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        })
    };
    scheduler.add_native_handler("heartbeat", heartbeat_handler).await;

    scheduler.add_native_handler(
        "hourly-report",
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
            }) as Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        }),
    ).await;

    scheduler.add_native_handler(
        "cache-cleanup",
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
            }) as Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        }),
    ).await;

    // 4. Load state and sync with configuration
    let _ = scheduler
        .load_tasks()
        .await
        .expect("Failed to load tasks from DB");

    if std::path::Path::new("cron.toml").exists() {
        println!("Syncing with cron.toml...");
        scheduler
            .reload_from_file("cron.toml")
            .await
            .expect("Failed to sync config");
    }

    // 5. Start the scheduler background loop
    scheduler.clone().start().await;

    // 6. Signal handling for reload
    let s_clone = Arc::clone(&scheduler);
    use tokio::signal::unix::{signal, SignalKind};
    let mut signals = signal(SignalKind::hangup()).expect("Failed to register SIGHUP handler");
    
    tokio::spawn(async move {
        loop {
            signals.recv().await;
            println!("SIGHUP received, reloading configuration...");
            match s_clone.reload_from_file("cron.toml").await {
                Ok(_) => println!("Configuration reloaded successfully."),
                Err(e) => eprintln!("Failed to reload config: {}", e),
            }
        }
    });

    // 7. Start the web server (blocking)
    let web_server = WebServer::new(Arc::clone(&scheduler), 9130);
    if let Err(e) = web_server.run().await {
        eprintln!("Web server error: {}", e);
    }
}
