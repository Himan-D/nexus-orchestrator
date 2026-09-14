use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::application::ports::{
    AssetRepository, JobRepository, RepositoryError, RunRepository, UserRepository,
};
use crate::domain::asset::{Asset, Materialization};
use crate::domain::job::Job;
use crate::domain::run::{Run, RunStatus, TaskRun, TaskStatus};
use crate::domain::user::User;

/// SQLite-backed implementation of every repository port. SQLite is the v1
/// store; the ports make Postgres a drop-in adapter later.
pub struct SqliteRepo {
    pool: SqlitePool,
}

fn db_err(e: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(anyhow::anyhow!(e.to_string()))
}

fn invalid(field: &str, e: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidData(format!("{field}: {e}"))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_dt(field: &'static str, s: &str) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| invalid(field, e))
}

fn parse_uuid(field: &'static str, s: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(s).map_err(|e| invalid(field, e))
}

fn parse_run_status(s: &str) -> Result<RunStatus, RepositoryError> {
    match s {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "success" => Ok(RunStatus::Success),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        other => Err(invalid("run.status", format!("unknown status '{other}'"))),
    }
}

fn parse_task_status(s: &str) -> Result<TaskStatus, RepositoryError> {
    match s {
        "pending" => Ok(TaskStatus::Pending),
        "running" => Ok(TaskStatus::Running),
        "success" => Ok(TaskStatus::Success),
        "failed" => Ok(TaskStatus::Failed),
        "skipped" => Ok(TaskStatus::Skipped),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(invalid("task_run.status", format!("unknown status '{other}'"))),
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .map(|d| d.is_unique_violation())
        .unwrap_or(false)
}

impl SqliteRepo {
    pub async fn connect(url: &str) -> Result<Self, RepositoryError> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(|e| RepositoryError::InvalidData(format!("bad db url: {e}")))?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10))
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await
            .map_err(db_err)?;
        Ok(Self { pool })
    }

    /// Single-connection in-memory database for tests (shared-cache issues
    /// make multi-connection `:memory:` pools unsafe).
    #[allow(dead_code)] // used by the #[cfg(test)] suites in this crate
    pub async fn in_memory() -> Result<Self, RepositoryError> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| RepositoryError::InvalidData(e.to_string()))?
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(db_err)?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), RepositoryError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| RepositoryError::Database(anyhow::anyhow!(e.to_string())))?;
        Ok(())
    }

    fn row_to_job(row: &SqliteRow) -> Result<Job, RepositoryError> {
        let tasks_str: String = row.get("tasks");
        let tasks = serde_json::from_str(&tasks_str)
            .map_err(|e| invalid("job.tasks", e))?;
        Ok(Job {
            id: parse_uuid("job.id", row.get::<String, _>("id").as_str())?,
            name: row.get("name"),
            description: row.get("description"),
            tasks,
            enabled: row.get::<i64, _>("enabled") != 0,
            schedule: row.get("schedule"),
            last_enqueued_at: row
                .get::<Option<String>, _>("last_enqueued_at")
                .map(|s| parse_dt("job.last_enqueued_at", &s))
                .transpose()?,
            owner_id: parse_uuid("job.owner_id", row.get::<String, _>("owner_id").as_str())?,
            created_at: parse_dt("job.created_at", row.get::<String, _>("created_at").as_str())?,
            updated_at: parse_dt("job.updated_at", row.get::<String, _>("updated_at").as_str())?,
        })
    }

    fn row_to_run(row: &SqliteRow) -> Result<Run, RepositoryError> {
        Ok(Run {
            id: parse_uuid("run.id", row.get::<String, _>("id").as_str())?,
            job_id: parse_uuid("run.job_id", row.get::<String, _>("job_id").as_str())?,
            job_name: row.get("job_name"),
            status: parse_run_status(row.get::<String, _>("status").as_str())?,
            triggered_by: row
                .get::<Option<String>, _>("triggered_by")
                .map(|s| parse_uuid("run.triggered_by", &s))
                .transpose()?,
            error: row.get("error"),
            created_at: parse_dt("run.created_at", row.get::<String, _>("created_at").as_str())?,
            started_at: row
                .get::<Option<String>, _>("started_at")
                .map(|s| parse_dt("run.started_at", &s))
                .transpose()?,
            finished_at: row
                .get::<Option<String>, _>("finished_at")
                .map(|s| parse_dt("run.finished_at", &s))
                .transpose()?,
        })
    }

    fn row_to_task_run(row: &SqliteRow) -> Result<TaskRun, RepositoryError> {
        Ok(TaskRun {
            id: parse_uuid("task_run.id", row.get::<String, _>("id").as_str())?,
            run_id: parse_uuid("task_run.run_id", row.get::<String, _>("run_id").as_str())?,
            task_id: row.get("task_id"),
            name: row.get("name"),
            status: parse_task_status(row.get::<String, _>("status").as_str())?,
            attempts: row.get::<i64, _>("attempts") as u32,
            materializes: row.get("materializes"),
            logs: row.get("logs"),
            error: row.get("error"),
            started_at: row
                .get::<Option<String>, _>("started_at")
                .map(|s| parse_dt("task_run.started_at", &s))
                .transpose()?,
            finished_at: row
                .get::<Option<String>, _>("finished_at")
                .map(|s| parse_dt("task_run.finished_at", &s))
                .transpose()?,
        })
    }

    fn row_to_asset(row: &SqliteRow) -> Result<Asset, RepositoryError> {
        Ok(Asset {
            key: row.get("key"),
            name: row.get("name"),
            description: row.get("description"),
            last_run_id: row
                .get::<Option<String>, _>("last_run_id")
                .map(|s| parse_uuid("asset.last_run_id", &s))
                .transpose()?,
            last_materialized_at: row
                .get::<Option<String>, _>("last_materialized_at")
                .map(|s| parse_dt("asset.last_materialized_at", &s))
                .transpose()?,
            created_at: parse_dt("asset.created_at", row.get::<String, _>("created_at").as_str())?,
        })
    }
}

#[async_trait]
impl JobRepository for SqliteRepo {
    async fn create(&self, job: &Job) -> Result<(), RepositoryError> {
        let tasks = serde_json::to_string(&job.tasks)
            .map_err(|e| RepositoryError::InvalidData(e.to_string()))?;
        sqlx::query("INSERT INTO jobs (id, name, description, tasks, enabled, schedule, last_enqueued_at, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(job.id.to_string())
            .bind(&job.name)
            .bind(&job.description)
            .bind(tasks)
            .bind(job.enabled as i64)
            .bind(&job.schedule)
            .bind(job.last_enqueued_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(job.owner_id.to_string())
            .bind(job.created_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .bind(job.updated_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn update(&self, job: &Job) -> Result<(), RepositoryError> {
        let tasks = serde_json::to_string(&job.tasks)
            .map_err(|e| RepositoryError::InvalidData(e.to_string()))?;
        let res = sqlx::query("UPDATE jobs SET name = ?, description = ?, tasks = ?, enabled = ?, schedule = ?, last_enqueued_at = ?, updated_at = ? WHERE id = ?")
            .bind(&job.name)
            .bind(&job.description)
            .bind(tasks)
            .bind(job.enabled as i64)
            .bind(&job.schedule)
            .bind(job.last_enqueued_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(now_rfc3339())
            .bind(job.id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Job>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(Self::row_to_job).transpose()
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Job>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM jobs ORDER BY created_at DESC LIMIT ? OFFSET ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(Self::row_to_job).collect()
    }

    async fn list_scheduled(&self) -> Result<Vec<Job>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM jobs WHERE schedule IS NOT NULL AND enabled = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(Self::row_to_job).collect()
    }

    async fn count(&self) -> Result<i64, RepositoryError> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM jobs")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.get::<i64, _>("n"))
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError> {
        let res = sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}

#[async_trait]
impl RunRepository for SqliteRepo {
    async fn create(&self, run: &Run) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO runs (id, job_id, job_name, status, triggered_by, error, created_at, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(run.id.to_string())
            .bind(run.job_id.to_string())
            .bind(&run.job_name)
            .bind(run.status.as_str())
            .bind(run.triggered_by.map(|u| u.to_string()))
            .bind(&run.error)
            .bind(run.created_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .bind(run.started_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(run.finished_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Run>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(Self::row_to_run).transpose()
    }

    async fn list(
        &self,
        status: Option<RunStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Run>, RepositoryError> {
        let rows = if let Some(status) = status {
            sqlx::query("SELECT * FROM runs WHERE status = ? ORDER BY created_at DESC LIMIT ? OFFSET ?")
                .bind(status.as_str())
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?
        } else {
            sqlx::query("SELECT * FROM runs ORDER BY created_at DESC LIMIT ? OFFSET ?")
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?
        };
        rows.iter().map(Self::row_to_run).collect()
    }

    async fn list_by_job(
        &self,
        job_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Run>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM runs WHERE job_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?")
            .bind(job_id.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(Self::row_to_run).collect()
    }

    async fn count_by_status(&self) -> Result<Vec<(RunStatus, i64)>, RepositoryError> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS n FROM runs GROUP BY status")
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                let status = parse_run_status(r.get::<String, _>("status").as_str())?;
                Ok((status, r.get::<i64, _>("n")))
            })
            .collect()
    }

    async fn claim_queued(&self, max: usize) -> Result<Vec<Run>, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let ids: Vec<String> = sqlx::query(
            "SELECT id FROM runs WHERE status = 'queued' ORDER BY created_at LIMIT ?",
        )
        .bind(max as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_err)?
        .iter()
        .map(|r| r.get::<String, _>("id"))
        .collect();

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let now = now_rfc3339();
        for id in &ids {
            sqlx::query("UPDATE runs SET status = 'running', started_at = ? WHERE id = ? AND status = 'queued'")
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;

        // One round-trip for all claimed runs (ids come from our own SELECT,
        // so interpolating placeholders is safe).
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!("SELECT * FROM runs WHERE id IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        rows.iter().map(Self::row_to_run).collect()
    }

    async fn fail_interrupted(&self) -> Result<u64, RepositoryError> {
        let now = now_rfc3339();
        sqlx::query("UPDATE task_runs SET status = 'failed', error = 'interrupted by process restart', finished_at = ? WHERE status = 'running'")
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;

        let res = sqlx::query("UPDATE runs SET status = 'failed', error = 'interrupted by process restart', finished_at = ? WHERE status = 'running'")
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn cancel_if_queued(&self, id: Uuid, reason: &str) -> Result<bool, RepositoryError> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'cancelled', error = ?, finished_at = ? WHERE id = ? AND status = 'queued'",
        )
        .bind(reason)
        .bind(now_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: RunStatus,
        error: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let finished = if status.is_terminal() { Some(now_rfc3339()) } else { None };
        let res = sqlx::query("UPDATE runs SET status = ?, error = ?, finished_at = COALESCE(?, finished_at) WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(finished)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }

    async fn create_task_run(&self, tr: &TaskRun) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO task_runs (id, run_id, task_id, name, status, attempts, materializes, logs, error, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(tr.id.to_string())
            .bind(tr.run_id.to_string())
            .bind(&tr.task_id)
            .bind(&tr.name)
            .bind(tr.status.as_str())
            .bind(tr.attempts as i64)
            .bind(&tr.materializes)
            .bind(&tr.logs)
            .bind(&tr.error)
            .bind(tr.started_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(tr.finished_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn update_task_run(&self, tr: &TaskRun) -> Result<(), RepositoryError> {
        let res = sqlx::query("UPDATE task_runs SET status = ?, attempts = ?, logs = ?, error = ?, started_at = COALESCE(?, started_at), finished_at = ? WHERE id = ?")
            .bind(tr.status.as_str())
            .bind(tr.attempts as i64)
            .bind(&tr.logs)
            .bind(&tr.error)
            .bind(tr.started_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(tr.finished_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(tr.id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }

    async fn task_runs(&self, run_id: Uuid) -> Result<Vec<TaskRun>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM task_runs WHERE run_id = ? ORDER BY rowid")
            .bind(run_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(Self::row_to_task_run).collect()
    }
}

#[async_trait]
impl AssetRepository for SqliteRepo {
    async fn upsert(&self, asset: &Asset) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO assets (key, name, description, last_run_id, last_materialized_at, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(key) DO UPDATE SET name = excluded.name, description = excluded.description")
            .bind(&asset.key)
            .bind(&asset.name)
            .bind(&asset.description)
            .bind(asset.last_run_id.map(|u| u.to_string()))
            .bind(asset.last_materialized_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Micros, true)))
            .bind(asset.created_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Asset>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM assets WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref().map(Self::row_to_asset).transpose()
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Asset>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM assets ORDER BY last_materialized_at DESC, key LIMIT ? OFFSET ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter().map(Self::row_to_asset).collect()
    }

    async fn count(&self) -> Result<i64, RepositoryError> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM assets")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.get::<i64, _>("n"))
    }

    async fn record_materialization(&self, m: &Materialization) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO asset_materializations (id, asset_key, run_id, task_run_id, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(m.id.to_string())
            .bind(&m.asset_key)
            .bind(m.run_id.to_string())
            .bind(m.task_run_id.to_string())
            .bind(m.created_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn materializations(
        &self,
        asset_key: &str,
        limit: i64,
    ) -> Result<Vec<Materialization>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM asset_materializations WHERE asset_key = ? ORDER BY created_at DESC LIMIT ?")
            .bind(asset_key)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok(Materialization {
                    id: parse_uuid("mat.id", r.get::<String, _>("id").as_str())?,
                    asset_key: r.get("asset_key"),
                    run_id: parse_uuid("mat.run_id", r.get::<String, _>("run_id").as_str())?,
                    task_run_id: parse_uuid("mat.task_run_id", r.get::<String, _>("task_run_id").as_str())?,
                    created_at: parse_dt("mat.created_at", r.get::<String, _>("created_at").as_str())?,
                })
            })
            .collect()
    }

    async fn touch_last_materialized(
        &self,
        key: &str,
        run_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE assets SET last_run_id = ?, last_materialized_at = ? WHERE key = ?")
            .bind(run_id.to_string())
            .bind(at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[async_trait]
impl UserRepository for SqliteRepo {
    async fn create(&self, user: &User) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO users (id, email, password_hash, created_at) VALUES (?, ?, ?, ?)")
            .bind(user.id.to_string())
            .bind(&user.email)
            .bind(&user.password_hash)
            .bind(user.created_at.to_rfc3339_opts(SecondsFormat::Micros, true))
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if is_unique_violation(&e) {
                    RepositoryError::Conflict(format!("email '{}' already registered", user.email))
                } else {
                    db_err(e)
                }
            })?;
        Ok(())
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<User>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref()
            .map(|r| {
                Ok(User {
                    id: parse_uuid("user.id", r.get::<String, _>("id").as_str())?,
                    email: r.get("email"),
                    password_hash: r.get("password_hash"),
                    created_at: parse_dt("user.created_at", r.get::<String, _>("created_at").as_str())?,
                })
            })
            .transpose()
    }

    async fn get(&self, id: Uuid) -> Result<Option<User>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        row.as_ref()
            .map(|r| {
                Ok(User {
                    id: parse_uuid("user.id", r.get::<String, _>("id").as_str())?,
                    email: r.get("email"),
                    password_hash: r.get("password_hash"),
                    created_at: parse_dt("user.created_at", r.get::<String, _>("created_at").as_str())?,
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::Task;

    async fn repo() -> SqliteRepo {
        let r = SqliteRepo::in_memory().await.unwrap();
        r.run_migrations().await.unwrap();
        r
    }

    /// Disambiguating views onto the multi-trait repo.
    fn jobs(r: &SqliteRepo) -> &dyn JobRepository { r }
    fn runs(r: &SqliteRepo) -> &dyn RunRepository { r }
    fn assets(r: &SqliteRepo) -> &dyn AssetRepository { r }
    fn users(r: &SqliteRepo) -> &dyn UserRepository { r }

    fn sample_job() -> Job {
        Job::new(
            "etl",
            "test job",
            vec![Task {
                id: "t1".into(),
                name: "T1".into(),
                command: "echo hi".into(),
                depends_on: vec![],
                asset_inputs: vec![],
                materializes: Some("a.b".into()),
                timeout_secs: 60,
                max_retries: 2,
                retry_delay_secs: 3,
            }],
            Uuid::new_v4(),
        )
    }

    #[tokio::test]
    async fn job_crud_roundtrip() {
        let repo = repo().await;
        let job = sample_job();
        jobs(&repo).create(&job).await.unwrap();

        let fetched = jobs(&repo).get(job.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "etl");
        assert_eq!(fetched.tasks.len(), 1);
        assert_eq!(fetched.tasks[0].max_retries, 2);
        assert!(fetched.enabled);

        let mut updated = fetched.clone();
        updated.enabled = false;
        updated.name = "renamed".into();
        jobs(&repo).update(&updated).await.unwrap();
        let refetched = jobs(&repo).get(job.id).await.unwrap().unwrap();
        assert!(!refetched.enabled);
        assert_eq!(refetched.name, "renamed");

        assert_eq!(jobs(&repo).count().await.unwrap(), 1);
        jobs(&repo).delete(job.id).await.unwrap();
        assert!(jobs(&repo).get(job.id).await.unwrap().is_none());
        assert!(matches!(jobs(&repo).delete(job.id).await, Err(RepositoryError::NotFound)));
    }

    #[tokio::test]
    async fn duplicate_email_is_conflict() {
        let repo = repo().await;
        let u = User::new("a@b.com", "hash");
        users(&repo).create(&u).await.unwrap();
        let u2 = User::new("a@b.com", "hash2");
        assert!(matches!(
            users(&repo).create(&u2).await,
            Err(RepositoryError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn run_lifecycle_and_task_runs() {
        let repo = repo().await;
        let job = sample_job();
        jobs(&repo).create(&job).await.unwrap();

        let run = Run::new(&job, Some(Uuid::new_v4()));
        runs(&repo).create(&run).await.unwrap();

        let claimed = runs(&repo).claim_queued(5).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].status, RunStatus::Running);
        assert!(claimed[0].started_at.is_some());

        runs(&repo).update_status(run.id, RunStatus::Failed, Some("boom")).await.unwrap();
        let done = runs(&repo).get(run.id).await.unwrap().unwrap();
        assert_eq!(done.status, RunStatus::Failed);
        assert!(done.finished_at.is_some());
        assert_eq!(done.error.as_deref(), Some("boom"));

        let tr = TaskRun::new(run.id, &job.tasks[0]);
        runs(&repo).create_task_run(&tr).await.unwrap();
        let mut tr2 = tr.clone();
        tr2.status = TaskStatus::Success;
        tr2.attempts = 1;
        tr2.logs = Some("out".into());
        runs(&repo).update_task_run(&tr2).await.unwrap();
        let stored = runs(&repo).task_runs(run.id).await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, TaskStatus::Success);
        assert_eq!(stored[0].logs.as_deref(), Some("out"));
    }

    #[tokio::test]
    async fn asset_upsert_and_materializations() {
        let repo = repo().await;
        let a = Asset::new("warehouse.users");
        assets(&repo).upsert(&a).await.unwrap();
        let run_id = Uuid::new_v4();
        let tr_id = Uuid::new_v4();
        assets(&repo)
            .record_materialization(&Materialization::new("warehouse.users", run_id, tr_id))
            .await
            .unwrap();
        assets(&repo)
            .touch_last_materialized("warehouse.users", run_id, Utc::now())
            .await
            .unwrap();

        let fetched = assets(&repo).get("warehouse.users").await.unwrap().unwrap();
        assert_eq!(fetched.last_run_id, Some(run_id));
        assert!(fetched.last_materialized_at.is_some());
        let mats = assets(&repo).materializations("warehouse.users", 10).await.unwrap();
        assert_eq!(mats.len(), 1);
        assert_eq!(mats[0].run_id, run_id);
    }

    #[tokio::test]
    async fn corrupted_row_is_error_not_panic() {
        let repo = repo().await;
        sqlx::query("INSERT INTO runs (id, job_id, job_name, status, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(Uuid::new_v4().to_string())
            .bind(Uuid::new_v4().to_string())
            .bind("bad")
            .bind("NOT_A_STATUS")
            .bind(now_rfc3339())
            .execute(&repo.pool)
            .await
            .unwrap();

        // Corrupt rows must surface as errors, never panic or silently map.
        assert!(runs(&repo).list(None, 10, 0).await.is_err());
        let rows = sqlx::query("SELECT * FROM runs").fetch_all(&repo.pool).await.unwrap();
        let parsed: Result<Vec<_>, _> = rows.iter().map(SqliteRepo::row_to_run).collect();
        assert!(parsed.is_err());
    }
}
