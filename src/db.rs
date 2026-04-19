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
                enabled BOOLEAN NOT NULL DEFAULT 1,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
                (),
            )
            .await?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS cron_logs (
                id TEXT PRIMARY KEY NOT NULL,
                task_id TEXT NOT NULL,
                run_at DATETIME NOT NULL,
                status TEXT NOT NULL,
                FOREIGN KEY(task_id) REFERENCES cron_tasks(id)
            )",
                (),
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
        enabled: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cron_tasks (id, name, cron_expr, timezone, task_type, payload, enabled) VALUES (?, ?, ?, ?, ?, ?, ?)",
            libsql::params![
                id.to_string(),
                name.to_string(),
                cron_expr.to_string(),
                timezone.to_string(),
                task_type.to_string(),
                payload.map(|s| s.to_string()),
                if enabled { 1 } else { 0 }
            ]
        )
        .await?;

        Ok(())
    }

    pub async fn get_tasks(
        &self,
    ) -> Result<
        Vec<(Uuid, String, String, String, String, Option<String>, bool)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let mut rows = self
            .conn
            .query("SELECT id, name, cron_expr, timezone, task_type, payload, enabled FROM cron_tasks", ())
            .await?;

        let mut tasks = Vec::new();
        while let Some(row) = rows.next().await? {
            let id_str: String = row.get(0)?;
            let name: String = row.get(1)?;
            let cron_expr: String = row.get(2)?;
            let timezone: String = row.get(3)?;
            let task_type: String = row.get(4)?;
            let payload: Option<String> = row.get(5)?;
            let enabled_int: i64 = row.get(6)?;

            if let Ok(id) = Uuid::parse_str(&id_str) {
                tasks.push((id, name, cron_expr, timezone, task_type, payload, enabled_int != 0));
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

    pub async fn log_execution(
        &self,
        task_id: Uuid,
        status: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO cron_logs (id, task_id, run_at, status) VALUES (?, ?, ?, ?)",
            libsql::params![
                Uuid::new_v4().to_string(),
                task_id.to_string(),
                Utc::now().to_rfc3339(),
                status.to_string()
            ]
        )
        .await?;

        Ok(())
    }
}
