-- Nexus core schema (SQLite). Versioned via sqlx::migrate!.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    tasks TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    owner_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    job_name TEXT NOT NULL,
    status TEXT NOT NULL,
    triggered_by TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_runs_job ON runs(job_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);

CREATE TABLE IF NOT EXISTS task_runs (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    materializes TEXT,
    logs TEXT,
    error TEXT,
    started_at TEXT,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_task_runs_run ON task_runs(run_id);

CREATE TABLE IF NOT EXISTS assets (
    key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    last_run_id TEXT,
    last_materialized_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS asset_materializations (
    id TEXT PRIMARY KEY,
    asset_key TEXT NOT NULL,
    run_id TEXT NOT NULL,
    task_run_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_mat_asset ON asset_materializations(asset_key, created_at DESC);
