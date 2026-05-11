// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::pin::Pin;
use std::future::Future;
use uuid::Uuid;
use crate::db::Db;
use crate::scheduler::Scheduler;
use crate::task::LogMessage;

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
fn heartbeat_handler(counter: Arc<AtomicU32>) -> Arc<dyn Fn(Uuid, i64, Db, tokio::sync::broadcast::Sender<LogMessage>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync> {
    Arc::new(move |log_id, task_id, db, log_sender| {
        let counter = Arc::clone(&counter);
        Box::pin(async move {
            let count = counter.fetch_add(1, Ordering::SeqCst);
            let raw_msg = format!("Heartbeat #{}", count + 1);
            let msg = format!("[heartbeat] {}", raw_msg);
            println!("{}", msg);
            let _ = db.save_log_line(log_id, "heartbeat", Some("master"), &raw_msg).await;
            let _ = log_sender.send(LogMessage { 
                task_id, 
                log_id: Some(log_id),
                prefix: Some("heartbeat".to_string()),
                hostname: Some("master".to_string()),
                text: msg 
            });
            Ok(())
        }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    })
}

/// Hourly report placeholder
fn hourly_report_handler(log_id: Uuid, task_id: i64, db: Db, log_sender: tokio::sync::broadcast::Sender<LogMessage>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {

    Box::pin(async move {
        let raw_msg = "Generating hourly report...";
        let msg = format!("[hourly-report] {}", raw_msg);
        println!("{}", msg);
        let _ = db.save_log_line(log_id, "hourly-report", Some("master"), raw_msg).await;
        let _ = log_sender.send(LogMessage { 
                task_id, 
                log_id: Some(log_id),
                prefix: Some("hourly-report".to_string()),
                hostname: Some("master".to_string()),
                text: msg 
            });
        Ok(())
    })
}

/// Cache cleanup placeholder
fn cache_cleanup_handler(log_id: Uuid, task_id: i64, db: Db, log_sender: tokio::sync::broadcast::Sender<LogMessage>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {

    Box::pin(async move {
        let raw_msg = "Cleaning up cache...";
        let msg = format!("[cache-cleanup] {}", raw_msg);
        println!("{}", msg);
        let _ = db.save_log_line(log_id, "cache-cleanup", Some("master"), raw_msg).await;
        let _ = log_sender.send(LogMessage { 
                task_id, 
                log_id: Some(log_id),
                prefix: Some("cache-cleanup".to_string()),
                hostname: Some("master".to_string()),
                text: msg 
            });
        Ok(())
    })
}
