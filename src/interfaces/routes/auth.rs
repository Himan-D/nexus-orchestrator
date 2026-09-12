use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde_json::json;

use crate::application::auth::AuthedUser;
use crate::interfaces::dto::{AuthRequest, AuthResponse, UserDto};
use crate::interfaces::error::ApiError;
use crate::interfaces::AppState;

pub async fn signup(
    State(state): State<AppState>,
    Json(payload): Json<AuthRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, token) = state.auth.signup(&payload.email, &payload.password).await?;
    Ok((
        StatusCode::CREATED,
        [(SET_COOKIE, state.auth.session_cookie(&token, state.cookie_secure))],
        Json(AuthResponse { user: UserDto { id: user.id, email: user.email } }),
    ))
}

pub async fn signin(
    State(state): State<AppState>,
    Json(payload): Json<AuthRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, token) = state.auth.signin(&payload.email, &payload.password).await?;
    Ok((
        StatusCode::OK,
        [(SET_COOKIE, state.auth.session_cookie(&token, state.cookie_secure))],
        Json(AuthResponse { user: UserDto { id: user.id, email: user.email } }),
    ))
}

/// Clears the session cookie. Open to any caller — clearing a cookie that
/// doesn't exist is harmless.
pub async fn signout(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(SET_COOKIE, AuthService::cleared_cookie(state.cookie_secure))],
        Json(json!({"ok": true})),
    )
}

pub async fn me(
    State(state): State<AppState>,
    Extension(user): Extension<AuthedUser>,
) -> Result<Json<AuthResponse>, ApiError> {
    let fresh = state.auth.me(user.id).await?;
    Ok(Json(AuthResponse { user: UserDto { id: fresh.id, email: fresh.email } }))
}

use crate::application::auth::AuthService;
