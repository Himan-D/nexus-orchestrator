use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::domain::run::RunStatus;
use crate::interfaces::dto::{ListResponse, RunDetailResponse, RunListQuery, RunResponse, TaskRunResponse};
use crate::interfaces::error::ApiError;
use crate::interfaces::AppState;

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<RunListQuery>,
) -> Result<Json<ListResponse<RunResponse>>, ApiError> {
    let status = match q.status.as_deref() {
        None | Some("") => None,
        Some(s) => Some(parse_status(s)?),
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let runs = state.runs.list(status, limit, offset).await?;
    Ok(Json(ListResponse {
        items: runs.iter().map(RunResponse::from_domain).collect(),
        total: None,
    }))
}

pub async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RunDetailResponse>, ApiError> {
    let run = state.runs.get(id).await?.ok_or_else(|| ApiError::not_found("run"))?;
    let task_runs = state.runs.task_runs(id).await?;
    Ok(Json(RunDetailResponse {
        run: RunResponse::from_domain(&run),
        tasks: task_runs.iter().map(TaskRunResponse::from_domain).collect(),
    }))
}

/// Cancels a run. Queued runs are cancelled atomically in the DB (racing the
/// scheduler's claim); running runs get their cancellation token fired and
/// finalize as cancelled by the run engine.
pub async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run = state.runs.get(id).await?.ok_or_else(|| ApiError::not_found("run"))?;

    if run.status.is_terminal() {
        return Err(ApiError::conflict(format!("run is already {}", run.status)));
    }

    if run.status == RunStatus::Queued {
        let cancelled = state.runs.cancel_if_queued(id, "cancelled by user").await?;
        if cancelled {
            return Ok(Json(serde_json::json!({"status": "cancelled"})));
        }
    }

    // Running: signal the in-flight token; the engine finalizes the status.
    if !state.hub.cancel(id) {
        tracing::warn!("Cancel requested for run {} not currently registered", id);
    }
    Ok(Json(serde_json::json!({"status": "cancelling"})))
}

fn parse_status(s: &str) -> Result<RunStatus, ApiError> {
    match s.to_lowercase().as_str() {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "success" => Ok(RunStatus::Success),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        _ => Err(ApiError::bad_request(format!(
            "unknown status '{s}'; expected queued|running|success|failed|cancelled"
        ))),
    }
}
