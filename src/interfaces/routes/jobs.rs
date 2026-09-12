use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use uuid::Uuid;

use crate::application::auth::AuthedUser;
use crate::domain::job::Job;
use crate::domain::run::Run;
use crate::interfaces::dto::{
    CreateJobRequest, JobResponse, ListResponse, PageQuery, RunResponse, UpdateJobRequest,
};
use crate::interfaces::error::ApiError;
use crate::interfaces::AppState;

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<AuthedUser>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<(StatusCode, Json<JobResponse>), ApiError> {
    let tasks = payload.tasks.into_iter().map(|t| t.into_domain()).collect();
    let job = Job::new(payload.name, payload.description, tasks, user.id);
    job.validate()?;
    state.jobs.create(&job).await?;
    Ok((StatusCode::CREATED, Json(JobResponse::from_domain(&job))))
}

pub async fn list(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<ListResponse<JobResponse>>, ApiError> {
    let (limit, offset) = page.clamp();
    let jobs = state.jobs.list(limit, offset).await?;
    let total = state.jobs.count().await?;
    Ok(Json(ListResponse {
        items: jobs.iter().map(JobResponse::from_domain).collect(),
        total: Some(total),
    }))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobResponse>, ApiError> {
    let job = state.jobs.get(id).await?.ok_or_else(|| ApiError::not_found("job"))?;
    Ok(Json(JobResponse::from_domain(&job)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateJobRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    let mut job = state.jobs.get(id).await?.ok_or_else(|| ApiError::not_found("job"))?;

    if let Some(name) = payload.name {
        job.name = name;
    }
    if let Some(description) = payload.description {
        job.description = description;
    }
    if let Some(enabled) = payload.enabled {
        job.enabled = enabled;
    }
    if let Some(tasks) = payload.tasks {
        job.tasks = tasks.into_iter().map(|t| t.into_domain()).collect();
    }
    job.updated_at = chrono::Utc::now();
    job.validate()?; // full revalidation after any change
    state.jobs.update(&job).await?;
    Ok(Json(JobResponse::from_domain(&job)))
}

/// Deleting a job does not touch historical runs — they keep a snapshot of
/// the job name, so audit history survives definition deletion.
pub async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.jobs.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn trigger(
    State(state): State<AppState>,
    Extension(user): Extension<AuthedUser>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<RunResponse>), ApiError> {
    let job = state.jobs.get(id).await?.ok_or_else(|| ApiError::not_found("job"))?;
    if !job.enabled {
        return Err(ApiError::conflict("job is paused; enable it before triggering"));
    }
    let run = Run::new(&job, Some(user.id));
    state.runs.create(&run).await?;
    Ok((StatusCode::ACCEPTED, Json(RunResponse::from_domain(&run))))
}

pub async fn list_job_runs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<ListResponse<RunResponse>>, ApiError> {
    // 404 if the job doesn't exist at all.
    state.jobs.get(id).await?.ok_or_else(|| ApiError::not_found("job"))?;
    let (limit, offset) = page.clamp();
    let runs = state.runs.list_by_job(id, limit, offset).await?;
    Ok(Json(ListResponse {
        items: runs.iter().map(RunResponse::from_domain).collect(),
        total: None,
    }))
}
