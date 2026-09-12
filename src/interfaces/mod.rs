pub mod dto;
pub mod error;
pub mod middleware;
pub mod routes;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::application::auth::AuthService;
use crate::application::ports::{AssetRepository, JobRepository, RunRepository};
use crate::application::run_manager::RunHub;

#[derive(Clone)]
pub struct AppState {
    pub jobs: Arc<dyn JobRepository>,
    pub runs: Arc<dyn RunRepository>,
    pub assets: Arc<dyn AssetRepository>,
    pub auth: Arc<AuthService>,
    pub hub: RunHub,
    pub cookie_secure: bool,
}

/// Assembles the HTTP surface: public routes (health, auth entry) and
/// `/v1/*` routes protected by the auth middleware.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/healthz", get(routes::health::healthz))
        .route("/readyz", get(routes::health::readyz))
        .route("/v1/auth/signup", post(routes::auth::signup))
        .route("/v1/auth/signin", post(routes::auth::signin));

    let protected = Router::new()
        .route("/v1/auth/me", get(routes::auth::me))
        .route("/v1/auth/signout", post(routes::auth::signout))
        .route("/v1/jobs", post(routes::jobs::create).get(routes::jobs::list))
        .route(
            "/v1/jobs/:id",
            get(routes::jobs::get_job)
                .patch(routes::jobs::update)
                .delete(routes::jobs::delete_job),
        )
        .route(
            "/v1/jobs/:id/runs",
            post(routes::jobs::trigger).get(routes::jobs::list_job_runs),
        )
        .route("/v1/runs", get(routes::runs::list))
        .route("/v1/runs/:id", get(routes::runs::get_run))
        .route("/v1/runs/:id/cancel", post(routes::runs::cancel))
        .route("/v1/assets", get(routes::assets::list))
        .route("/v1/assets/:key", get(routes::assets::detail))
        .route("/v1/stats", get(routes::stats))
        .layer(axum::middleware::from_fn_with_state(
            state.auth.clone(),
            middleware::require_auth,
        ));

    public.merge(protected).with_state(state)
}
