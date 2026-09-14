use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::job::{Job, Task};
use crate::domain::run::{Run, RunStatus, TaskRun, TaskStatus};

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tasks: Vec<TaskDto>,
    /// Optional cron expression (UTC) for automatic runs.
    #[serde(default)]
    pub schedule: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJobRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub tasks: Option<Vec<TaskDto>>,
    pub schedule: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskDto {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub asset_inputs: Vec<String>,
    #[serde(default)]
    pub materializes: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub retry_delay_secs: Option<u64>,
}

impl TaskDto {
    pub fn into_domain(self) -> Task {
        Task {
            id: self.id,
            name: self.name,
            command: self.command,
            depends_on: self.depends_on,
            asset_inputs: self.asset_inputs,
            materializes: self.materializes,
            timeout_secs: self.timeout_secs.unwrap_or(crate::domain::job::DEFAULT_TASK_TIMEOUT_SECS),
            max_retries: self.max_retries.unwrap_or(0),
            retry_delay_secs: self.retry_delay_secs.unwrap_or(5),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl PageQuery {
    pub fn clamp(&self) -> (i64, i64) {
        let limit = self.limit.unwrap_or(50).clamp(1, 200);
        let offset = self.offset.unwrap_or(0).max(0);
        (limit, offset)
    }
}

#[derive(Debug, Deserialize)]
pub struct RunListQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub user: UserDto,
}

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub tasks: Vec<Task>,
    pub enabled: bool,
    pub schedule: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl JobResponse {
    pub fn from_domain(j: &Job) -> Self {
        Self {
            id: j.id,
            name: j.name.clone(),
            description: j.description.clone(),
            tasks: j.tasks.clone(),
            enabled: j.enabled,
            schedule: j.schedule.clone(),
            created_at: j.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            updated_at: j.updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RunResponse {
    pub id: Uuid,
    pub job_id: Uuid,
    pub job_name: String,
    pub status: RunStatus,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl RunResponse {
    pub fn from_domain(r: &Run) -> Self {
        let rfc = |d: Option<chrono::DateTime<chrono::Utc>>| {
            d.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        };
        Self {
            id: r.id,
            job_id: r.job_id,
            job_name: r.job_name.clone(),
            status: r.status,
            error: r.error.clone(),
            created_at: r.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            started_at: rfc(r.started_at),
            finished_at: rfc(r.finished_at),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TaskRunResponse {
    pub id: Uuid,
    pub task_id: String,
    pub name: String,
    pub status: TaskStatus,
    pub attempts: u32,
    pub materializes: Option<String>,
    pub logs: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl TaskRunResponse {
    pub fn from_domain(t: &TaskRun) -> Self {
        let rfc = |d: Option<chrono::DateTime<chrono::Utc>>| {
            d.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        };
        Self {
            id: t.id,
            task_id: t.task_id.clone(),
            name: t.name.clone(),
            status: t.status,
            attempts: t.attempts,
            materializes: t.materializes.clone(),
            logs: t.logs.clone(),
            error: t.error.clone(),
            started_at: rfc(t.started_at),
            finished_at: rfc(t.finished_at),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RunDetailResponse {
    #[serde(flatten)]
    pub run: RunResponse,
    pub tasks: Vec<TaskRunResponse>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: Option<i64>,
}
