-- Copyright 2026 Seungjin Kim
-- SPDX-License-Identifier: MIT OR Apache-2.0

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
    FOREIGN KEY(task_id) REFERENCES lachuoi_tasks(id) ON DELETE CASCADE
);

-- Task Output Logs Table
CREATE TABLE IF NOT EXISTS lachuoi_outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    log_id TEXT NOT NULL,
    module TEXT NOT NULL,
    host TEXT,
    output TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(log_id) REFERENCES lachuoi_logs(id) ON DELETE CASCADE
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
    remote_addr TEXT,
    headers TEXT,
    body TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Task Execution Events (WebSocket JSON-RPC logs)
CREATE TABLE IF NOT EXISTS lachuoi_task_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker_id TEXT,
    worker_hostname TEXT,
    direction TEXT NOT NULL, -- 'master_to_worker' or 'worker_to_master'
    method TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Kv_store Table
CREATE TABLE `lachuoi_kv_store` (
	`key` text PRIMARY KEY,
	`value` text,
	`created_at` numeric DEFAULT CURRENT_TIMESTAMP,
	`updated_at` numeric DEFAULT CURRENT_TIMESTAMP
);

-- Migrations (for existing databases)
-- Run these if you are upgrading from an older version
-- ALTER TABLE lachuoi_webhooks ADD COLUMN remote_addr TEXT;
-- ALTER TABLE lachuoi_outputs ADD COLUMN host TEXT;


