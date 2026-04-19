use chrono::Utc;
use libsql::{Builder, Connection};
use uuid::Uuid;

#[derive(Clone)]
pub struct Db {
    conn: Connection,
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

        let db_wrapper = Self { conn };
        db_wrapper.init().await?;

        // Migration: add columns if not exists
        let _ = db_wrapper.conn.execute("ALTER TABLE cron_tasks ADD COLUMN args TEXT", ()).await;
        let _ = db_wrapper.conn.execute("ALTER TABLE cron_tasks ADD COLUMN sha256 TEXT", ()).await;

        Ok(db_wrapper)
    }

    async fn init(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS cron_tasks (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                cron_expr TEXT NOT NULL,
                timezone TEXT NOT NULL,
                task_type TEXT NOT NULL DEFAULT 'native',
                payload TEXT,
                args TEXT,
                sha256 TEXT,
                enabled BOOLEAN NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
                (),
            )
            .await?;

        // Recreate logs with duration instead of status
        // We use duration_ms (NULL = not finished/crashed)
        let _ = self.conn.execute("DROP TABLE IF EXISTS cron_outputs", ()).await;
        let _ = self.conn.execute("DROP TABLE IF EXISTS cron_logs", ()).await;
        
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS cron_logs (
                id TEXT PRIMARY KEY NOT NULL,
                task_id TEXT NOT NULL,
                run_at DATETIME NOT NULL,
                duration_ms INTEGER,
                FOREIGN KEY(task_id) REFERENCES cron_tasks(id)
            )",
                (),
            )
            .await?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS cron_outputs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                log_id TEXT NOT NULL,
                output TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(log_id) REFERENCES cron_logs(id)
            )",
                (),
            )
            .await?;

        Ok(())
    }

    pub async fn save_log_line(
        &self,
        log_id: Uuid,
        output: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO cron_outputs (log_id, output) VALUES (?, ?)",
            libsql::params![
                log_id.to_string(),
                output.to_string()
            ]
        )
        .await?;

        Ok(())
    }

    pub async fn save_task(
        &self,
        id: Uuid,
        name: &str,
        cron_expr: &str,
        timezone: &str,
        task_type: &str,
        payload: Option<&str>,
        args: Option<Vec<String>>,
        sha256: Option<&str>,
        enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let args_json = args.map(|a| serde_json::to_string(&a).unwrap());

        self.conn.execute(
            "INSERT OR REPLACE INTO cron_tasks (id, name, cron_expr, timezone, task_type, payload, args, sha256, enabled) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            libsql::params![
                id.to_string(),
                name.to_string(),
                cron_expr.to_string(),
                timezone.to_string(),
                task_type.to_string(),
                payload.map(|s| s.to_string()),
                args_json,
                sha256.map(|s| s.to_string()),
                if enabled { 1 } else { 0 }
            ]
        )
        .await?;

        Ok(())
    }

    pub async fn get_tasks(
        &self,
    ) -> Result<
        Vec<(Uuid, String, String, String, String, Option<String>, Option<Vec<String>>, Option<String>, bool)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let mut rows = self
            .conn
            .query("SELECT id, name, cron_expr, timezone, task_type, payload, args, sha256, enabled FROM cron_tasks", ())
            .await?;

        let mut tasks = Vec::new();
        while let Some(row) = rows.next().await? {
            let id_str: String = row.get(0)?;
            let name: String = row.get(1)?;
            let cron_expr: String = row.get(2)?;
            let timezone: String = row.get(3)?;
            let task_type: String = row.get(4)?;
            let payload: Option<String> = row.get(5)?;
            let args_json: Option<String> = row.get(6)?;
            let sha256: Option<String> = row.get(7)?;
            let enabled_int: i64 = row.get(8)?;

            let args = args_json.and_then(|j| serde_json::from_str(&j).ok());

            if let Ok(id) = Uuid::parse_str(&id_str) {
                tasks.push((id, name, cron_expr, timezone, task_type, payload, args, sha256, enabled_int != 0));
            }
        }

        Ok(tasks)
    }

    pub async fn is_empty(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self
            .conn
            .query("SELECT COUNT(*) FROM cron_tasks", ())
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
        id: Uuid,
        enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn
            .execute(
                "UPDATE cron_tasks SET enabled = ? WHERE id = ?",
                libsql::params![if enabled { 1 } else { 0 }, id.to_string()],
            )
            .await?;
        Ok(())
    }

    pub async fn remove_task(
        &self,
        id: Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn
            .execute(
                "DELETE FROM cron_tasks WHERE id = ?",
                libsql::params![id.to_string()],
            )
            .await?;
        Ok(())
    }

    pub async fn log_execution_start(
        &self,
        task_id: Uuid,
    ) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
        let log_id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO cron_logs (id, task_id, run_at, duration_ms) VALUES (?, ?, ?, NULL)",
            libsql::params![
                log_id.to_string(),
                task_id.to_string(),
                Utc::now().to_rfc3339()
            ]
        )
        .await?;

        Ok(log_id)
    }

    pub async fn log_execution_finish(
        &self,
        log_id: Uuid,
        duration_ms: u128,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "UPDATE cron_logs SET duration_ms = ? WHERE id = ?",
            libsql::params![
                duration_ms as i64,
                log_id.to_string()
            ]
        )
        .await?;

        Ok(())
    }
}
