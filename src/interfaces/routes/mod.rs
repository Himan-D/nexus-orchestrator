pub mod assets;
pub mod auth;
pub mod health;
pub mod jobs;
pub mod runs;

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::application::lineage::{self, LineageGraph};
use crate::domain::run::RunStatus;
use crate::interfaces::{error::ApiError, AppState};

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub jobs: i64,
    pub assets: i64,
    pub runs_by_status: std::collections::BTreeMap<String, i64>,
}

pub async fn stats(
    State(state): State<AppState>,
) -> Result<Json<StatsResponse>, ApiError> {
    let jobs = state.jobs.count().await?;
    let assets = state.assets.count().await?;
    let by_status = state.runs.count_by_status().await?;
    let mut runs_by_status = std::collections::BTreeMap::new();
    for (status, n) in by_status {
        runs_by_status.insert(status.to_string(), n);
    }
    // Ensure every status key exists so the UI can render stable cards.
    for s in [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Success,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        runs_by_status.entry(s.to_string()).or_insert(0);
    }
    Ok(Json(StatsResponse { jobs, assets, runs_by_status }))
}

/// The asset dependency graph derived from pipeline definitions, with
/// per-asset staleness (upstream fresher than downstream).
pub async fn lineage_route(
    State(state): State<AppState>,
) -> Result<Json<LineageGraph>, ApiError> {
    let jobs = state.jobs.list(1000, 0).await?;
    let assets = state.assets.list(2000, 0).await?;
    Ok(Json(lineage::compute_lineage(&jobs, &assets)))
}
