use std::future::Future;
use tokio::time::Duration;

pub struct TaskConfig {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub timeout_ms: u64,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay_ms: 1000,
            timeout_ms: 30000,
        }
    }
}

// Execute a task with retries on failure
async fn execute_with_retry<F, Fut>(
    task_name: &str,
    config: &TaskConfig,
    handler: F,
) -> Result<(), String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut attempts = 0;
    let mut delay = config.retry_delay_ms;

    loop {
        attempts += 1;

        match tokio::time::timeout(
            Duration::from_millis(config.timeout_ms),
            handler(),
        )
        .await
        {
            Ok(Ok(())) => {
                return Ok(());
            }
            Ok(Err(e)) => {
                println!(
                    "Task '{}' failed (attempt {}): {}",
                    task_name, attempts, e
                );
            }
            Err(_) => {
                println!(
                    "Task '{}' timed out (attempt {})",
                    task_name, attempts
                );
            }
        }

        if attempts >= config.max_retries {
            return Err(format!(
                "Task '{}' failed after {} attempts",
                task_name, attempts
            ));
        }

        // Exponential backoff
        tokio::time::sleep(Duration::from_millis(delay)).await;
        delay *= 2;
    }
}
