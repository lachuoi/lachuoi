-- User Authorization Table
CREATE TABLE IF NOT EXISTS cron_users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    github_login TEXT UNIQUE NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Tasks Configuration Table
CREATE TABLE IF NOT EXISTS cron_tasks (
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
CREATE TABLE IF NOT EXISTS cron_logs (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    run_at DATETIME NOT NULL,
    duration_ms INTEGER,
    FOREIGN KEY(task_id) REFERENCES cron_tasks(id)
);

-- Task Output Logs Table
CREATE TABLE IF NOT EXISTS cron_outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    log_id TEXT NOT NULL,
    module TEXT NOT NULL,
    output TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(log_id) REFERENCES cron_logs(id)
);

-- Web Session Table
CREATE TABLE IF NOT EXISTS cron_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    record BLOB NOT NULL,
    expiry_date INTEGER NOT NULL
);

-- Webhook Logs Table
CREATE TABLE IF NOT EXISTS cron_webhooks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    headers TEXT,
    body TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Initial Authorized User
INSERT OR IGNORE INTO cron_users (github_login) VALUES ('seungjin');

-- Migrations
-- Add env column to cron_tasks if it doesn't exist (SQLite doesn't support IF NOT EXISTS for columns in ALTER TABLE directly)
-- User should run this manually if upgrading from an older version
-- ALTER TABLE cron_tasks ADD COLUMN env TEXT;
