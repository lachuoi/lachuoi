// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use chrono::Utc;
use libsql::{Builder, Connection};
use uuid::Uuid;
use tower_sessions::{session::{Id, Record}, SessionStore};
use async_trait::async_trait;
use std::collections::HashMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct SchemaConfig {
    version: i64,
    sql: String,
    #[serde(default)]
    migrations: Vec<Migration>,
}

#[derive(Deserialize)]
struct Migration {
    version: i64,
    sql: String,
}

#[derive(Clone)]
pub struct Db {
    conn: Connection,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").finish()
    }
}

impl Db {
    pub async fn new(
        url: &str,
        auth_token: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let db = if let Some(token) = auth_token {
            Builder::new_remote(url.to_string(), token.to_string())
                .build()
                .await?
        } else {
            Builder::new_local(url).build().await?
        };

        let conn = db.connect()?;
        
        // Ensure WAL mode for concurrent access
        if auth_token.is_none() {
            // PRAGMA statements often return metadata rows, which cause execute() to fail in libsql.
            // We use query() and ignore the results to safely apply these settings.
            let _ = conn.query("PRAGMA journal_mode = WAL", ()).await?;
            let _ = conn.query("PRAGMA synchronous = NORMAL", ()).await?;
            let _ = conn.query("PRAGMA busy_timeout = 5000", ()).await?;
        }

        let db_instance = Self { conn };
        db_instance.setup_schema().await?;

        Ok(db_instance)
    }

    async fn setup_schema(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::io::{self, Write};
        let schema_toml = include_str!("../schema.sql.toml");
        let config: SchemaConfig = toml::from_str(schema_toml)?;

        println!("  -> Initializing database schema (v{})...", config.version);
        io::stdout().flush().ok();

        // 1. Determine current version
        let mut current_version: i64 = 0;

        // Try kv_store first
        let has_kv_table = {
            let mut rows = self.conn.query("SELECT name FROM sqlite_master WHERE type='table' AND name='kv_store'", ()).await?;
            rows.next().await?.is_some()
        };

        if has_kv_table {
            let mut rows = self.conn.query("SELECT value FROM kv_store WHERE key='db_schema_version'", ()).await?;
            if let Some(row) = rows.next().await? {
                let v: String = row.get(0)?;
                current_version = v.parse().unwrap_or(0);
            }
        }

        println!("  -> Current database version: {}", current_version);
        io::stdout().flush().ok();

        // 2. Fresh install or Upgrade check
        if current_version == 0 {
            println!("  -> Performing fresh database installation...");
            io::stdout().flush().ok();
            // Fresh install: Run the full schema
            for statement in config.sql.split(';') {
                let s = statement.trim();
                if !s.is_empty() {
                    self.conn.execute(s, ()).await?;
                }
            }
            println!("  -> Database installation complete.");
        } else if current_version < config.version {
            println!("  -> Upgrading database from v{} to v{}...", current_version, config.version);
            io::stdout().flush().ok();
            
            let mut applied_any = false;
            for m in config.migrations {
                if m.version > current_version {
                    println!("    -> Applying migration v{}...", m.version);
                    io::stdout().flush().ok();
                    for statement in m.sql.split(';') {
                        let s = statement.trim();
                        if !s.is_empty() {
                            self.conn.execute(s, ()).await?;
                        }
                    }
                    
                    // Update version in kv_store
                    self.conn.execute(
                        "INSERT OR REPLACE INTO kv_store (key, value) VALUES ('db_schema_version', ?)", 
                        libsql::params![m.version.to_string()]
                    ).await?;
                    applied_any = true;
                }
            }

            if !applied_any {
                println!("  -> No specific migrations found, performing version-only update.");
                self.conn.execute(
                    "INSERT OR REPLACE INTO kv_store (key, value) VALUES ('db_schema_version', ?)", 
                    libsql::params![config.version.to_string()]
                ).await?;
            }
            println!("  -> Database upgrade complete.");
        } else {
            println!("  -> Database is up to date.");
        }

        // 3. Ensure ADMIN_USER is authorized
        if let Ok(admin) = std::env::var("ADMIN_USER") {
            if !admin.trim().is_empty() {
                self.conn.execute(
                    "INSERT OR IGNORE INTO users (github_login) VALUES (?)",
                    libsql::params![admin.trim()]
                ).await?;
                println!("  -> Verified authorization for ADMIN_USER: {}", admin.trim());
            }
        }

        io::stdout().flush().ok();

        Ok(())
    }

    pub async fn get_app_key_values(&self, app_id: i64, key: &str) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT value FROM task_kv_store WHERE app_id = ? AND key = ? ORDER BY created_at ASC",
            libsql::params![app_id, key]
        ).await?;

        let mut values = Vec::new();
        while let Some(row) = rows.next().await? {
            let v: String = row.get(0)?;
            values.push(v);
        }
        Ok(values)
    }

    pub async fn add_app_key_value(&self, app_id: i64, key: &str, value: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO task_kv_store (app_id, key, value) VALUES (?, ?, ?)",
            libsql::params![app_id, key, value]
        ).await?;
        Ok(())
    }

    pub async fn is_authorized(
        &self,
        github_login: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self
            .conn
            .query(
                "SELECT COUNT(*) FROM users WHERE github_login = ?",
                libsql::params![github_login],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let count: i64 = row.get(0)?;
            Ok(count > 0)
        } else {
            Ok(false)
        }
    }

    pub async fn save_log_line(
        &self,
        log_id: Uuid,
        module: &str,
        host: Option<&str>,
        output: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO outputs (log_id, module, host, output) VALUES (?, ?, ?, ?)",
            libsql::params![
                log_id.to_string(),
                module.to_string(),
                host,
                output.to_string()
            ]
        )
        .await?;

        Ok(())
    }

    pub async fn save_task(
        &self,
        id: i64,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        task_type: &str,
        payload: Option<&str>,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
        sha256: Option<&str>,
        enabled: bool,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let args_json = args.map(|a| serde_json::to_string(&a).unwrap());
        let env_json = env.map(|e| serde_json::to_string(&e).unwrap());

        if id == 0 {
            // Insert new task
            self.conn.execute(
                "INSERT INTO tasks (name, cron_expr, timezone, task_type, payload, args, env, sha256, enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                libsql::params![
                    name.to_string(),
                    cron_expr.to_string(),
                    timezone.to_string(),
                    task_type.to_string(),
                    payload.map(|s| s.to_string()),
                    args_json,
                    env_json,
                    sha256.map(|s| s.to_string()),
                    if enabled { 1 } else { 0 }
                ]
            ).await?;
            Ok(self.conn.last_insert_rowid())
        } else {
            // Update or replace existing task
            self.conn.execute(
                "INSERT OR REPLACE INTO tasks (id, name, cron_expr, timezone, task_type, payload, args, env, sha256, enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                libsql::params![
                    id,
                    name.to_string(),
                    cron_expr.to_string(),
                    timezone.to_string(),
                    task_type.to_string(),
                    payload.map(|s| s.to_string()),
                    args_json,
                    env_json,
                    sha256.map(|s| s.to_string()),
                    if enabled { 1 } else { 0 }
                ]
            ).await?;
            Ok(id)
        }
    }

    pub async fn get_tasks(
        &self,
    ) -> Result<
        Vec<(i64, String, String, String, String, Option<String>, Option<Vec<String>>, Option<HashMap<String, String>>, Option<String>, bool)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let mut rows = self
            .conn
            .query("SELECT id, name, cron_expr, timezone, task_type, payload, args, env, sha256, enabled FROM tasks", ())
            .await?;

        let mut tasks = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let cron_expr: String = row.get(2)?;
            let timezone: String = row.get(3)?;
            let task_type: String = row.get(4)?;
            let payload: Option<String> = row.get(5)?;
            let args_json: Option<String> = row.get(6)?;
            let env_json: Option<String> = row.get(7)?;
            let sha256: Option<String> = row.get(8)?;
            let enabled_int: i64 = row.get(9)?;

            let args = args_json.and_then(|j| serde_json::from_str(&j).ok());
            let env = env_json.and_then(|j| serde_json::from_str(&j).ok());

            tasks.push((id, name, cron_expr, timezone, task_type, payload, args, env, sha256, enabled_int != 0));
        }

        Ok(tasks)
    }

    pub async fn is_empty(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self
            .conn
            .query("SELECT COUNT(*) FROM tasks", ())
            .await?;
        if let Some(row) = rows.next().await? {
            let count: i64 = row.get(0)?;
            Ok(count == 0)
        } else {
            Ok(true)
        }
    }

    pub async fn update_task_enabled(
        &self,
        id: i64,
        enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn
            .execute(
                "UPDATE tasks SET enabled = ? WHERE id = ?",
                libsql::params![if enabled { 1 } else { 0 }, id],
            )
            .await?;
        Ok(())
    }

    pub async fn remove_task(
        &self,
        id: i64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 1. Delete outputs associated with logs of this task
        self.conn.execute(
            "DELETE FROM outputs WHERE log_id IN (SELECT id FROM logs WHERE task_id = ?)",
            libsql::params![id]
        ).await?;

        // 2. Delete logs of this task
        self.conn.execute(
            "DELETE FROM logs WHERE task_id = ?",
            libsql::params![id]
        ).await?;

        // 3. Delete the task itself
        self.conn.execute(
            "DELETE FROM tasks WHERE id = ?",
            libsql::params![id]
        ).await?;

        Ok(())
    }

    pub async fn log_execution_start(
        &self,
        task_id: i64,
    ) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
        let log_id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO logs (id, task_id, run_at, duration_ms) VALUES (?, ?, ?, NULL)",
            libsql::params![
                log_id.to_string(),
                task_id,
                Utc::now().to_rfc3339()
            ]
        )
        .await?;

        Ok(log_id)
    }

    pub async fn log_execution_finish(
        &self,
        log_id: Uuid,
        duration_ms: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "UPDATE logs SET duration_ms = ? WHERE id = ?",
            libsql::params![
                duration_ms as i64,
                log_id.to_string()
            ]
        )
        .await?;

        Ok(())
    }

    pub fn get_conn(&self) -> Connection {
        self.conn.clone()
    }

    pub async fn get_latest_task_logs(
        &self,
    ) -> Result<std::collections::HashMap<i64, (Uuid, String, Option<i64>)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT id, task_id, run_at, duration_ms FROM logs WHERE (task_id, run_at) IN (SELECT task_id, MAX(run_at) FROM logs WHERE duration_ms IS NOT NULL GROUP BY task_id)",
            ()
        ).await?;

        let mut results = std::collections::HashMap::new();
        while let Some(row) = rows.next().await? {
            let id_str: String = row.get(0)?;
            let task_id: i64 = row.get(1)?;
            let run_at: String = row.get(2)?;
            let duration_ms: Option<i64> = row.get(3)?;

            if let Ok(log_id) = Uuid::parse_str(&id_str) {
                results.insert(task_id, (log_id, run_at, duration_ms));
            }
        }
        Ok(results)
    }

    pub async fn get_logs_by_id(
        &self,
        log_id: Uuid,
    ) -> Result<Vec<(Option<i64>, String, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT l.task_id, o.output, o.created_at FROM outputs o JOIN logs l ON o.log_id = l.id WHERE o.log_id = ? ORDER BY o.created_at ASC",
            libsql::params![log_id.to_string()]
        ).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let task_id: Option<i64> = row.get(0).ok();
            results.push((task_id, row.get(1)?, row.get(2)?));
        }
        Ok(results)
    }

    pub async fn get_initial_outputs(
        &self,
        limit: usize,
    ) -> Result<Vec<(Option<i64>, String, Option<String>, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT l.task_id, o.output, o.host, o.created_at FROM outputs o JOIN logs l ON o.log_id = l.id ORDER BY o.created_at DESC LIMIT ?",
            libsql::params![limit as i64]
        ).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let task_id: Option<i64> = row.get(0).ok();
            let output: String = row.get(1)?;
            let host: Option<String> = row.get(2)?;
            let created_at: String = row.get(3)?;
            results.push((task_id, output, host, created_at));
        }
        results.reverse();
        Ok(results)
    }

    pub async fn save_webhook(
        &self,
        path: &str,
        method: &str,
        remote_addr: Option<&str>,
        headers: &str,
        body: &str,
    ) -> Result<WebhookLog, Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO webhooks (path, method, remote_addr, headers, body) VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                path.to_string(),
                method.to_string(),
                remote_addr.map(|s| s.to_string()),
                headers.to_string(),
                body.to_string()
            ]
        )
        .await?;

        let row_id = self.conn.last_insert_rowid();
        let created_at = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        Ok(WebhookLog {
            id: row_id,
            path: path.to_string(),
            method: method.to_string(),
            remote_addr: remote_addr.map(|s| s.to_string()),
            headers: headers.to_string(),
            body: body.to_string(),
            created_at,
        })
    }

    pub async fn get_webhooks(&self) -> Result<Vec<WebhookLog>, Box<dyn std::error::Error + Send + Sync>> {
        self.get_webhooks_paginated(100, 0).await
    }

    pub async fn get_webhooks_paginated(&self, limit: usize, offset: usize) -> Result<Vec<WebhookLog>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT id, path, method, remote_addr, headers, body, created_at FROM webhooks ORDER BY created_at DESC LIMIT ? OFFSET ?",
            libsql::params![limit as i64, offset as i64]
        ).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(WebhookLog {
                id: row.get(0)?,
                path: row.get(1)?,
                method: row.get(2)?,
                remote_addr: row.get(3)?,
                headers: row.get(4)?,
                body: row.get(5)?,
                created_at: row.get(6)?,
            });
        }
        Ok(results)
    }

    pub async fn get_webhooks_count(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT COUNT(*) FROM webhooks",
            ()
        ).await?;

        if let Some(row) = rows.next().await? {
            let count: i64 = row.get(0)?;
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }

    pub async fn delete_webhook(&self, id: i64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "DELETE FROM webhooks WHERE id = ?",
            libsql::params![id]
        ).await?;
        Ok(())
    }

    pub async fn save_task_log(
        &self,
        worker_id: Option<&str>,
        worker_hostname: Option<&str>,
        direction: &str,
        method: &str,
        payload: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO task_logs (worker_id, worker_hostname, direction, method, payload) VALUES (?, ?, ?, ?, ?)",
            libsql::params![
                worker_id,
                worker_hostname,
                direction.to_string(),
                method.to_string(),
                payload.to_string()
            ]
        )
        .await?;
        Ok(())
    }

    pub async fn get_task_logs_paginated(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::task::TaskLogEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT id, worker_id, worker_hostname, direction, method, payload, created_at FROM task_logs ORDER BY created_at DESC LIMIT ? OFFSET ?",
            libsql::params![limit as i64, offset as i64]
        ).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(crate::task::TaskLogEntry {
                id: row.get(0)?,
                worker_id: row.get(1)?,
                worker_hostname: row.get(2)?,
                direction: row.get(3)?,
                method: row.get(4)?,
                payload: row.get(5)?,
                created_at: row.get(6)?,
            });
        }
        Ok(results)
    }

    pub async fn get_task_logs_count(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT COUNT(*) FROM task_logs",
            ()
        ).await?;

        if let Some(row) = rows.next().await? {
            let count: i64 = row.get(0)?;
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }

    pub async fn get_run_logs(&self, log_id: Uuid) -> Result<Vec<(String, Option<String>, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT module, host, output FROM outputs WHERE log_id = ? ORDER BY id ASC",
            libsql::params![log_id.to_string()]
        ).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn setup_db() -> Db {
        let db = Db::new(":memory:", None).await.unwrap();
        db.conn.execute("PRAGMA foreign_keys = ON;", ()).await.unwrap();
        db
    }


    #[tokio::test]
    async fn test_remove_task_with_logs() {
        let db = setup_db().await;
        let task_name = "test_task";

        // 1. Create a task
        let task_id = db.save_task(
            0,
            task_name,
            "* * * * *",
            "UTC",
            "native",
            None,
            None,
            None,
            None,
            true
        ).await.unwrap();

        // 2. Create a log for the task
        let log_id = db.log_execution_start(task_id).await.unwrap();
        db.log_execution_finish(log_id, 100).await.unwrap();

        // 3. Create an output for the log
        db.save_log_line(log_id, "test_module", None, "test_output").await.unwrap();

        // 4. Attempt to remove the task
        db.remove_task(task_id).await.expect("Failed to remove task with logs");

        // 5. Verify task is gone
        let tasks = db.get_tasks().await.unwrap();
        assert!(tasks.iter().all(|(id, _, _, _, _, _, _, _, _, _)| *id != task_id));

        // 6. Verify logs and outputs are gone (indirectly, but good to check)
        let mut rows = db.conn.query("SELECT COUNT(*) FROM logs WHERE task_id = ?", libsql::params![task_id]).await.unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(count, 0);

        let mut rows = db.conn.query("SELECT COUNT(*) FROM outputs WHERE log_id = ?", libsql::params![log_id.to_string()]).await.unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(count, 0);
    }
}

#[derive(Clone, serde::Serialize)]
pub struct WebhookLog {
    pub id: i64,
    pub path: String,
    pub method: String,
    pub remote_addr: Option<String>,
    pub headers: String,
    pub body: String,
    pub created_at: String,
}

#[async_trait]
impl SessionStore for Db {
    async fn save(&self, record: &Record) -> tower_sessions::session_store::Result<()> {
        let record_data = serde_json::to_vec(record)
            .map_err(|e| tower_sessions::session_store::Error::Encode(e.to_string()))?;
        
        let expiry = record.expiry_date.unix_timestamp();

        self.conn.execute(
            "INSERT OR REPLACE INTO sessions (id, record, expiry_date) VALUES (?, ?, ?)",
            libsql::params![record.id.to_string(), record_data, expiry]
        ).await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, id: &Id) -> tower_sessions::session_store::Result<Option<Record>> {
        let mut rows = self.conn.query(
            "SELECT record FROM sessions WHERE id = ? AND expiry_date > ?",
            libsql::params![id.to_string(), Utc::now().timestamp()]
        ).await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;

        if let Some(row) = rows.next().await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))? {
            let record_data: Vec<u8> = row.get(0).map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;
            let record: Record = serde_json::from_slice(&record_data)
                .map_err(|e| tower_sessions::session_store::Error::Decode(e.to_string()))?;
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, id: &Id) -> tower_sessions::session_store::Result<()> {
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?",
            libsql::params![id.to_string()]
        ).await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;

        Ok(())
    }

    async fn create(&self, record: &mut Record) -> tower_sessions::session_store::Result<()> {
        self.save(record).await
    }
}
