// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;
use std::io::{self, Write};
use lachuoi::db::Db;
use lachuoi::scheduler::Scheduler;
use lachuoi::web::WebServer;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let pid_file = ".scheduler.pid";

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h" || args[1] == "help") {
        println!("La Chuoi - Distributed WASI Runtime & Service Framework");
        println!();
        println!("Usage: lachuoi [COMMAND]");
        println!();
        println!("Commands:");
        println!("  (none)      Start the Master node server");
        println!("  reload      Reload configuration from cron.toml (zero-downtime)");
        println!("  migrate     Run database migrations and authorize ADMIN_USER");
        println!("  help        Display this help message");
        println!();
        println!("Environment Variables:");
        println!("  TURSO_DATABASE_URL    Database URL or local file path (default: lachuoi.db)");
        println!("  TURSO_AUTH_TOKEN      Auth token for remote Turso DB");
        println!("  NODE_KEY              Shared secret key for Worker connections");
        println!("  ENVIRONMENT           'development' or 'production' (default: production)");
        println!("  PORT                  Web server port (default: 9130)");
        println!("  ADMIN_USER            GitHub login of the initial administrator");
        println!("  LACHUOI_PUBLIC_URL    Public URL of the master node for task callbacks");
        return;
    }

    if args.len() > 1 && (args[1] == "reload" || args[1] == "--reload") {
        use tokio::process::Command;
        
        let pid_str = match std::fs::read_to_string(pid_file) {
            Ok(content) => content.trim().to_string(),
            Err(_) => {
                eprintln!("Error: PID file '{}' not found. Is the Master node running?", pid_file);
                return;
            }
        };

        println!("Sending reload signal (SIGHUP) to Master node (PID {})...", pid_str);
        let output = Command::new("kill")
            .arg("-HUP")
            .arg(&pid_str)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => println!("Reload signal sent successfully."),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                eprintln!("Error sending signal: {}", err);
            }
            Err(e) => eprintln!("Failed to execute kill command: {}", e),
        }
        return;
    }

    let env = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "production".to_string());
    println!("Starting La Chuoi in {} mode...", env);

    if args.len() > 1 && args[1] == "migrate" {
        let db_url = std::env::var("TURSO_DATABASE_URL")
            .unwrap_or_else(|_| "lachuoi.db".to_string());
        
        let auth_token = if db_url.starts_with("libsql://") {
            std::env::var("TURSO_AUTH_TOKEN").ok()
        } else {
            None
        };

        println!("Running database migrations...");
        match Db::new(&db_url, auth_token.as_deref()).await {
            Ok(_) => println!("Migrations completed successfully."),
            Err(e) => {
                eprintln!("Migration error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // Write PID file for the server process
    if let Err(e) = std::fs::write(pid_file, std::process::id().to_string()) {
        eprintln!("Warning: Could not write PID file: {}", e);
    }

    // 1. Initialize Database
    let db_url = std::env::var("TURSO_DATABASE_URL")
        .unwrap_or_else(|_| "lachuoi.db".to_string());
    
    let auth_token = if db_url.starts_with("libsql://") {
        std::env::var("TURSO_AUTH_TOKEN").ok()
    } else {
        None
    };

    println!("Connecting to database at {}...", db_url);
    io::stdout().flush().ok();
    let db = Db::new(&db_url, auth_token.as_deref())
        .await
        .expect("Failed to initialize database");
    println!("Database initialized successfully.");
    io::stdout().flush().ok();

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
            println!("Received SIGHUP, reloading configuration...");
            match s_clone.reload_from_file("cron.toml").await {
                Ok(_) => println!("Configuration reloaded successfully."),
                Err(e) => eprintln!("Failed to reload config: {}", e),
            }
            // Broadcast update immediately
            s_clone.broadcast_status().await;
        }
    });

    // 7. Start Web Server
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9130);

    let web_server = WebServer::new(Arc::clone(&scheduler), db.clone(), port);
    if let Err(e) = web_server.run().await {
        eprintln!("Web server error: {}", e);
    }
}
