use chrono::Utc;
use libsql::{Builder, Connection};
use uuid::Uuid;
use tower_sessions::{session::{Id, Record}, SessionStore};
use async_trait::async_trait;

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

        Ok(Self { conn })
    }

    pub async fn is_authorized(
        &self,
        github_login: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self
            .conn
            .query(
                "SELECT COUNT(*) FROM cron_users WHERE github_login = ?",
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
        output: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.conn.execute(
            "INSERT INTO cron_outputs (log_id, module, output) VALUES (?, ?, ?)",
            libsql::params![
                log_id.to_string(),
                module.to_string(),
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
        duration_ms: u64,
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

    pub fn get_conn(&self) -> Connection {
        self.conn.clone()
    }

    pub async fn get_latest_task_logs(
        &self,
    ) -> Result<std::collections::HashMap<Uuid, (String, Option<i64>)>, Box<dyn std::error::Error + Send + Sync>> {
        let mut rows = self.conn.query(
            "SELECT task_id, run_at, duration_ms FROM cron_logs WHERE (task_id, run_at) IN (SELECT task_id, MAX(run_at) FROM cron_logs WHERE duration_ms IS NOT NULL GROUP BY task_id)",
            ()
        ).await?;

        let mut results = std::collections::HashMap::new();
        while let Some(row) = rows.next().await? {
            let task_id_str: String = row.get(0)?;
            let run_at: String = row.get(1)?;
            let duration_ms: Option<i64> = row.get(2)?;

            if let Ok(task_id) = Uuid::parse_str(&task_id_str) {
                results.insert(task_id, (run_at, duration_ms));
            }
        }
        Ok(results)
    }
}

#[async_trait]
impl SessionStore for Db {
    async fn save(&self, record: &Record) -> tower_sessions::session_store::Result<()> {
        let record_data = serde_json::to_vec(record)
            .map_err(|e| tower_sessions::session_store::Error::Encode(e.to_string()))?;
        
        let expiry = record.expiry_date.unix_timestamp();

        self.conn.execute(
            "INSERT OR REPLACE INTO cron_sessions (id, record, expiry_date) VALUES (?, ?, ?)",
            libsql::params![record.id.to_string(), record_data, expiry]
        ).await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, id: &Id) -> tower_sessions::session_store::Result<Option<Record>> {
        let mut rows = self.conn.query(
            "SELECT record FROM cron_sessions WHERE id = ? AND expiry_date > ?",
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
            "DELETE FROM cron_sessions WHERE id = ?",
            libsql::params![id.to_string()]
        ).await.map_err(|e| tower_sessions::session_store::Error::Backend(e.to_string()))?;

        Ok(())
    }

    async fn create(&self, record: &mut Record) -> tower_sessions::session_store::Result<()> {
        self.save(record).await
    }
}
