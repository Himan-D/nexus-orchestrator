use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::interfaces::error::ApiError;
use crate::interfaces::AppState;

/// Liveness: the process is up. Always cheap, never touches the DB.
pub async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

/// Readiness: the DB answers. Used by orchestrators (compose, k8s) to gate
/// traffic until migrations and the pool are actually usable.
pub async fn readyz(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    state
        .runs
        .count_by_status()
        .await
        .map(|_| Json(json!({"status": "ready"})))
        .map_err(|e| {
            tracing::error!("Readiness check failed: {e}");
            ApiError::service_unavailable("database unavailable")
        })
}
