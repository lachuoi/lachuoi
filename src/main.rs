use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use task_scheduler::scheduler::Scheduler;

#[tokio::main]
async fn main() {
    let scheduler = Scheduler::new();

    // Counter to track executions
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    // Run every minute
    scheduler
        .add_task(
            "heartbeat",
            "0 * * * * *", // Every minute at second 0
            move || {
                let count = counter_clone.fetch_add(1, Ordering::SeqCst);
                println!(
                    "[{}] Heartbeat #{}",
                    Utc::now().format("%H:%M:%S"),
                    count + 1
                );
            },
        )
        .await
        .expect("Failed to add heartbeat task");

    // Run at the top of every hour
    scheduler
        .add_task(
            "hourly-report",
            "0 0 * * * *", // At minute 0, second 0 of every hour
            || {
                println!(
                    "[{}] Generating hourly report...",
                    Utc::now().format("%H:%M:%S")
                );
            },
        )
        .await
        .expect("Failed to add hourly report task");

    // Run every 5 minutes
    scheduler
        .add_task(
            "cache-cleanup",
            "0 */5 * * * *", // Every 5 minutes at second 0
            || {
                println!(
                    "[{}] Cleaning up cache...",
                    Utc::now().format("%H:%M:%S")
                );
            },
        )
        .await
        .expect("Failed to add cache cleanup task");

    // Start the scheduler
    scheduler.start().await;
}
