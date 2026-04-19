use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    #[serde(rename = "task")]
    pub tasks: Vec<TaskConfig>,
}

#[derive(Debug, Deserialize)]
pub struct TaskConfig {
    pub name: String,
    pub cron: String,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub payload: Option<String>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

impl AppConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }
}
