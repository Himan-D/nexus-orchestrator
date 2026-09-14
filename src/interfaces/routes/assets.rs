use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Serialize;

use crate::application::lineage;
use crate::interfaces::dto::{ListResponse, PageQuery};
use crate::interfaces::error::ApiError;
use crate::interfaces::AppState;

#[derive(Debug, Serialize)]
pub struct AssetResponse {
    pub key: String,
    pub name: String,
    pub description: String,
    pub last_run_id: Option<uuid::Uuid>,
    pub last_materialized_at: Option<String>,
    /// True when an upstream input is fresher than this asset (or it has
    /// never been materialized) — the "needs re-run" signal.
    pub stale: bool,
}

impl AssetResponse {
    fn from_domain(a: &crate::domain::asset::Asset, stale: bool) -> Self {
        Self {
            key: a.key.clone(),
            name: a.name.clone(),
            description: a.description.clone(),
            last_run_id: a.last_run_id,
            last_materialized_at: a
                .last_materialized_at
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            stale,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AssetDetailResponse {
    #[serde(flatten)]
    pub asset: AssetResponse,
    pub materializations: Vec<MaterializationResponse>,
}

#[derive(Debug, Serialize)]
pub struct MaterializationResponse {
    pub id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub created_at: String,
}

pub async fn list(
    State(state): State<AppState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<ListResponse<AssetResponse>>, ApiError> {
    let (limit, offset) = page.clamp();
    let assets = state.assets.list(limit, offset).await?;
    let total = state.assets.count().await?;
    // Staleness is derived from lineage over pipeline definitions.
    let jobs = state.jobs.list(1000, 0).await?;
    let graph = lineage::compute_lineage(&jobs, &assets);
    let stale: HashMap<String, bool> = graph
        .nodes
        .into_iter()
        .map(|n| (n.key, n.stale))
        .collect();
    Ok(Json(ListResponse {
        items: assets
            .iter()
            .map(|a| {
                AssetResponse::from_domain(a, stale.get(&a.key).copied().unwrap_or(false))
            })
            .collect(),
        total: Some(total),
    }))
}

/// Returns one asset with its recent materialization history. The key is a
/// single path segment; URL-encode keys containing `/` or `:`.
pub async fn detail(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<AssetDetailResponse>, ApiError> {
    let asset = state
        .assets
        .get(&key)
        .await?
        .ok_or_else(|| ApiError::not_found("asset"))?;
    let mats = state.assets.materializations(&key, 50).await?;
    Ok(Json(AssetDetailResponse {
        asset: AssetResponse::from_domain(&asset, false),
        materializations: mats
            .iter()
            .map(|m| MaterializationResponse {
                id: m.id,
                run_id: m.run_id,
                created_at: m.created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .collect(),
    }))
}
