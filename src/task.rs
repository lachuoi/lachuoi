use chrono::{DateTime, Utc, Duration};
use chrono_tz::Tz;
use cron::Schedule;
use std::collections::HashMap;
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
    pub last_log_id: Option<Uuid>,
    pub host: Option<String>,
    pub last_host: Option<String>,
}


#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SystemMetrics {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub uptime: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct WorkerInfo {
    pub id: Uuid,
    pub addr: String,
    pub hostname: String,
    pub running_tasks: Vec<String>,
    pub metrics: Option<SystemMetrics>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct LogMessage {
    pub task_id: Uuid,
    pub log_id: Option<Uuid>,
    pub prefix: Option<String>,
    pub hostname: Option<String>,
    pub text: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RunRequest {
    pub wasm_path: String,
    pub expected_sha256: Option<String>,
    pub task_name: String,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub log_id: Uuid,
    pub task_id: Uuid,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
    pub id: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct TaskLogEntry {
    pub id: i64,
    pub worker_id: Option<String>,
    pub worker_hostname: Option<String>,
    pub direction: String,
    pub method: String,
    pub payload: String,
    pub created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MasterMessage {
    Bootstrap(BootstrapInfo),
    WasmBegin {
        path: String,
        total_size: usize,
    },
    WasmChunk {
        path: String,
        chunk: String, // Hex encoded chunk
        offset: usize,
    },
    WasmEnd {
        path: String,
    },
    RunTask(RunRequest),
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct BootstrapInfo {
    pub config_toml: String,
    pub wasm_paths: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    Log(LogMessage),
    GetWasm {
        path: String,
    },
    Metrics(SystemMetrics),
    TaskStarted {
        task_id: Uuid,
        task_name: String,
    },
    TaskResult {
        task_id: Uuid,
        log_id: Uuid,
        success: bool,
        error: Option<String>,
    },
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
    pub env: Option<HashMap<String, String>>,
    pub sha256: Option<String>,
    pub last_run: Option<DateTime<Utc>>,
    pub last_duration: Option<u64>,
    pub last_failed: bool,
    pub enabled: bool,
    pub last_log_id: Option<Uuid>,
    pub last_host: Option<String>,
}

impl ScheduledTask {
    pub fn new(name: &str, cron_expr: &str, timezone: &str) -> Result<Self, String> {
        let tz = timezone.parse::<Tz>().map_err(|e| format!("Invalid timezone {}: {}", timezone, e))?;
        Ok(Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            timezone: tz,
            task_type: "native".to_string(),
            payload: None,
            args: None,
            env: None,
            sha256: None,
            last_run: None,
            last_duration: None,
            last_failed: false,
            enabled: true,
            last_log_id: None,
            last_host: None,
        })
    }

    pub fn new_wasm(name: &str, cron_expr: &str, timezone: &str, wasm_path: &str, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, sha256: Option<String>) -> Result<Self, String> {
        let tz = timezone.parse::<Tz>().map_err(|e| format!("Invalid timezone {}: {}", timezone, e))?;
        Ok(Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            cron_expr: cron_expr.to_string(),
            timezone: tz,
            task_type: "wasm".to_string(),
            payload: Some(wasm_path.to_string()),
            args,
            env,
            sha256,
            last_run: None,
            last_duration: None,
            last_failed: false,
            enabled: true,
            last_log_id: None,
            last_host: None,
        })
    }

    pub fn from_db(id: Uuid, name: String, cron_expr: String, timezone: String, task_type: String, payload: Option<String>, args: Option<Vec<String>>, env: Option<HashMap<String, String>>, sha256: Option<String>, enabled: bool) -> Result<Self, String> {
        let tz = timezone.parse::<Tz>().map_err(|e| format!("Invalid timezone {}: {}", timezone, e))?;
        Ok(Self {
            id,
            name,
            cron_expr,
            timezone: tz,
            task_type,
            payload,
            args,
            env,
            sha256,
            last_run: None,
            last_duration: None,
            last_failed: false,
            enabled,
            last_log_id: None,
            last_host: None,
        })
    }

    pub fn config_equals(&self, cron_expr: &str, timezone: &str, task_type: &str, payload: Option<&str>, args: Option<&[String]>, env: Option<&HashMap<String, String>>, sha256: Option<&str>) -> bool {
        self.cron_expr == cron_expr &&
        self.timezone.to_string() == timezone &&
        self.task_type == task_type &&
        self.payload.as_deref() == payload &&
        self.args.as_deref() == args &&
        self.env.as_ref() == env &&
        self.sha256.as_deref() == sha256
    }

    pub fn should_run(&self) -> bool {
        if !self.enabled {
            return false;
        }

        let schedule = match Schedule::from_str(&self.cron_expr) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let now = Utc::now().with_timezone(&self.timezone);
        let last_run = match self.last_run {
            Some(lr) => lr.with_timezone(&self.timezone),
            None => return true, // Never run before
        };

        // Find the most recent occurrence that should have happened
        if let Some(next) = schedule.after(&last_run).next() {
            if next <= now {
                return true;
            }
        }

        false
    }
}
