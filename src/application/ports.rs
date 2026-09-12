use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::domain::asset::{Asset, Materialization};
use crate::domain::job::Job;
use crate::domain::run::{Run, RunStatus, TaskRun};
use crate::domain::user::User;

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid data: {0}")]
    InvalidData(String),
    #[error("database error")]
    Database(#[source] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Jobs (pipeline definitions)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait JobRepository: Send + Sync {
    async fn create(&self, job: &Job) -> Result<(), RepositoryError>;
    async fn update(&self, job: &Job) -> Result<(), RepositoryError>;
    async fn get(&self, id: Uuid) -> Result<Option<Job>, RepositoryError>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Job>, RepositoryError>;
    async fn count(&self) -> Result<i64, RepositoryError>;
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError>;
}

// ---------------------------------------------------------------------------
// Runs (executions of jobs)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait RunRepository: Send + Sync {
    async fn create(&self, run: &Run) -> Result<(), RepositoryError>;
    async fn get(&self, id: Uuid) -> Result<Option<Run>, RepositoryError>;
    async fn list(
        &self,
        status: Option<RunStatus>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Run>, RepositoryError>;
    async fn list_by_job(
        &self,
        job_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Run>, RepositoryError>;
    async fn count_by_status(&self) -> Result<Vec<(RunStatus, i64)>, RepositoryError>;

    /// Atomically claims up to `max` queued runs by flipping them to Running
    /// inside a transaction, so two schedulers can never double-dispatch.
    async fn claim_queued(&self, max: usize) -> Result<Vec<Run>, RepositoryError>;

    /// Startup recovery: anything left Running by a previous process is
    /// failed as interrupted, so runs never hang forever after a crash.
    async fn fail_interrupted(&self) -> Result<u64, RepositoryError>;

    /// Atomically cancels a run that is still queued. Returns false when the
    /// run was not in `queued` state (already claimed, or terminal).
    async fn cancel_if_queued(&self, id: Uuid, reason: &str) -> Result<bool, RepositoryError>;

    /// Moves a run to a new status (engine-owned transitions) and stamps
    /// `finished_at` when the status is terminal.
    async fn update_status(
        &self,
        id: Uuid,
        status: RunStatus,
        error: Option<&str>,
    ) -> Result<(), RepositoryError>;

    async fn create_task_run(&self, task_run: &TaskRun) -> Result<(), RepositoryError>;
    async fn update_task_run(&self, task_run: &TaskRun) -> Result<(), RepositoryError>;
    async fn task_runs(&self, run_id: Uuid) -> Result<Vec<TaskRun>, RepositoryError>;
}

// ---------------------------------------------------------------------------
// Assets (the data a pipeline produces)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AssetRepository: Send + Sync {
    async fn upsert(&self, asset: &Asset) -> Result<(), RepositoryError>;
    async fn get(&self, key: &str) -> Result<Option<Asset>, RepositoryError>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Asset>, RepositoryError>;
    async fn count(&self) -> Result<i64, RepositoryError>;
    async fn record_materialization(&self, m: &Materialization) -> Result<(), RepositoryError>;
    async fn materializations(
        &self,
        asset_key: &str,
        limit: i64,
    ) -> Result<Vec<Materialization>, RepositoryError>;
    /// Updates the freshness pointer after a successful materialization.
    async fn touch_last_materialized(
        &self,
        key: &str,
        run_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
}

// ---------------------------------------------------------------------------
// Users & auth
// ---------------------------------------------------------------------------

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn create(&self, user: &User) -> Result<(), RepositoryError>;
    async fn get_by_email(&self, email: &str) -> Result<Option<User>, RepositoryError>;
    async fn get(&self, id: Uuid) -> Result<Option<User>, RepositoryError>;
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TaskOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("task timed out")]
    Timeout,
    #[error("task cancelled")]
    Cancelled,
    #[error("failed to spawn process: {0}")]
    Spawn(String),
}

/// Executes a single task command. Sandboxing policy belongs to the adapter
/// (local shell today, a container or K8s pod tomorrow) — the application
/// layer only sees `TaskOutput`.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(
        &self,
        command: &str,
        timeout: std::time::Duration,
        cancel: CancellationToken,
    ) -> Result<TaskOutput, ExecutorError>;
}
