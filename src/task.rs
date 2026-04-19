use chrono::{DateTime, Utc};
use cron::Schedule;
use std::str::FromStr;
use uuid::Uuid;

// Represents a scheduled task with its cron schedule and execution logic
pub struct ScheduledTask {
    pub id: Uuid,
    pub name: String,
    pub schedule: Schedule,
    pub last_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

impl ScheduledTask {
    // Create a new task from a cron expression string
    pub fn new(
        name: &str,
        cron_expr: &str,
    ) -> Result<Self, cron::error::Error> {
        let schedule = Schedule::from_str(cron_expr)?;

        Ok(Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            schedule,
            last_run: None,
            enabled: true,
        })
    }

    // Get the next scheduled execution time
    pub fn next_run(&self) -> Option<DateTime<Utc>> {
        self.schedule.upcoming(Utc).next()
    }

    // Check if the task should run now
    pub fn should_run(&self) -> bool {
        if !self.enabled {
            return false;
        }

        let now = Utc::now();

        // Find the most recent scheduled time
        if let Some(upcoming) = self
            .schedule
            .after(&(now - chrono::Duration::minutes(1)))
            .next()
        {
            // Task should run if we're within the same minute as a scheduled time
            return upcoming <= now;
        }

        false
    }
}
