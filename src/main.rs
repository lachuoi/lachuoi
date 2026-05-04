// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;
use lachuoi::db::Db;
use lachuoi::scheduler::Scheduler;
use lachuoi::web::WebServer;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    // Fix for rustls 0.23+ CryptoProvider panic
    let _ = rustls::crypto::ring::default_provider().install_default();

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

    // 3. Register native and WASM handlers
    lachuoi::native_handlers::register_all(&scheduler).await;
    lachuoi::wasm_handlers::register_all(&scheduler).await;

    // 4. Sync with cron.toml (DB only) then load state
    if std::path::Path::new("cron.toml").exists() {
        println!("Syncing database with cron.toml...");
        let config = lachuoi::config::AppConfig::load("cron.toml")
            .expect("Failed to load cron.toml");
        scheduler
            .sync_db_only(&config)
            .await
            .expect("Failed to sync DB with config");
    }

    scheduler
        .load_tasks()
        .await
        .expect("Failed to load tasks from DB");

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
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9130);

    let web_server = WebServer::new(Arc::clone(&scheduler), db.clone(), port);
    if let Err(e) = web_server.run().await {
        eprintln!("Web server error: {}", e);
    }
}
