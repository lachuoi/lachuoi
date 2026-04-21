-- User Authorization Table
CREATE TABLE IF NOT EXISTS lachuoi_users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    github_login TEXT UNIQUE NOT NULL,
    github_avatar_url TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Tasks Configuration Table
CREATE TABLE IF NOT EXISTS lachuoi_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    cron_expr TEXT NOT NULL,
    timezone TEXT NOT NULL,
    task_type TEXT NOT NULL DEFAULT 'native',
    payload TEXT,
    args TEXT,
    env TEXT,
    sha256 TEXT,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Execution Logs Table
CREATE TABLE IF NOT EXISTS lachuoi_logs (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    run_at DATETIME NOT NULL,
    duration_ms INTEGER,
    FOREIGN KEY(task_id) REFERENCES lachuoi_tasks(id)
);

-- Task Output Logs Table
CREATE TABLE IF NOT EXISTS lachuoi_outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    log_id TEXT NOT NULL,
    module TEXT NOT NULL,
    output TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(log_id) REFERENCES lachuoi_logs(id)
);

-- Web Session Table
CREATE TABLE IF NOT EXISTS lachuoi_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    record BLOB NOT NULL,
    expiry_date INTEGER NOT NULL
);

-- Webhook Logs Table
CREATE TABLE IF NOT EXISTS lachuoi_webhooks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    headers TEXT,
    body TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);


