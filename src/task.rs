use chrono::{DateTime, Utc, Duration};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;
use uuid::Uuid;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TaskStatus {
    pub id: Uuid,
    pub name: String,
    pub cron: String,
    pub timezone: String,
    pub task_type: String,
    pub last_run: Option<String>,
    pub last_duration_ms: Option<u64>,
    pub last_failed: bool,
    pub enabled: bool,
}

// Represents a scheduled task with its cron schedule and execution logic
pub struct ScheduledTask {
    pub id: Uuid,
    pub name: String,
    pub cron_expr: String,
    pub timezone: Tz,
    pub task_type: String,
    pub payload: Option<String>,
    pub args: Option<Vec<String>>,
    pub sha256: Option<String>,
    pub schedule: Schedule,
    pub last_run: Option<DateTime<Tz>>,
    pub last_duration: Option<u64>,
    pub last_failed: bool,
    pub enabled: bool,
}

impl ScheduledTask {
    // Create a new native task
    pub fn new(
        name: &str,
        cron_expr: &str,
        timezone_str: &str,
    ) -> Result<Self, String> {
        let schedule = Schedule::from_str(cron_expr)
            .map_err(|e| format!("Invalid cron expression: {}", e))?;
        
        let timezone = timezone_str.parse::<Tz>()
            .map_err(|e| format!("Invalid timezone '{}': {}", timezone_str, e))?;

        Ok(Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            timezone,
            task_type: "native".to_string(),
            payload: None,
            args: None,
            sha256: None,
            schedule,
            last_run: None,
            last_duration: None,
            last_failed: false,
            enabled: true,
        })
    }

    // Create a new WASM task
    pub fn new_wasm(
        name: &str,
        cron_expr: &str,
        timezone_str: &str,
        wasm_path: &str,
        args: Option<Vec<String>>,
        sha256: Option<String>,
    ) -> Result<Self, String> {
        let mut task = Self::new(name, cron_expr, timezone_str)?;
        task.task_type = "wasm".to_string();
        task.payload = Some(wasm_path.to_string());
        task.args = args;
        task.sha256 = sha256;
        Ok(task)
    }

    // Constructor for loading from DB
    pub fn from_db(
        id: Uuid,
        name: String,
        cron_expr: String,
        timezone: Tz,
        task_type: String,
        payload: Option<String>,
        args: Option<Vec<String>>,
        sha256: Option<String>,
        enabled: bool,
    ) -> Result<Self, String> {
        let schedule = Schedule::from_str(&cron_expr)
            .map_err(|e| format!("Invalid cron expression: {}", e))?;

        Ok(Self {
            id,
            name,
            cron_expr,
            timezone,
            task_type,
            payload,
            args,
            sha256,
            schedule,
            last_run: None,
            last_duration: None,
            last_failed: false,
            enabled,
        })
    }

    // Get the next scheduled execution time in the task's timezone
    pub fn next_run(&self) -> Option<DateTime<Tz>> {
        self.schedule.upcoming(self.timezone).next()
    }

    // Check if the task should run now based on its timezone and last run
    pub fn should_run(&self) -> bool {
        if !self.enabled {
            return false;
        }

        let now = Utc::now().with_timezone(&self.timezone);
        
        // Use last_run as the starting point, or now - 1 minute if it never ran
        // We subtract a small buffer (1ms) to ensure that if a task is scheduled exactly on the second,
        // and last_run was that exact second, we still find the NEXT occurrence.
        let start_time = self.last_run.unwrap_or_else(|| now - Duration::minutes(1));

        if let Some(next_occurrence) = self.schedule.after(&start_time).next() {
            // If the next scheduled occurrence is in the past or exactly now, it's time to run.
            return next_occurrence <= now;
        }

        false
    }

    pub fn config_equals(&self, cron: &str, timezone: &str, task_type: &str, payload: Option<&str>, args: Option<&[String]>, sha256: Option<&str>) -> bool {
        self.cron_expr == cron &&
        self.timezone.to_string() == timezone &&
        self.task_type == task_type &&
        self.payload.as_deref() == payload &&
        self.args.as_deref() == args &&
        self.sha256.as_deref() == sha256
    }
}
