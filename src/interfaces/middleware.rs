use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;

use crate::application::auth::AuthService;
use crate::interfaces::error::ApiError;


/// Extracts and verifies the session from `Authorization: Bearer <jwt>` or
/// the httpOnly session cookie, then injects `AuthedUser` into extensions.
pub async fn require_auth(
    State(auth): State<Arc<AuthService>>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer_token(&req)
        .or_else(|| cookie_token(&req, AuthService::cookie_name()))
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;

    let user = auth.verify_token(&token)?;
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

fn bearer_token(req: &Request) -> Option<String> {
    let value = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn cookie_token(req: &Request, name: &str) -> Option<String> {
    let value = req.headers().get(axum::http::header::COOKIE)?.to_str().ok()?;
    value.split(';').find_map(|pair| {
        let mut kv = pair.trim().splitn(2, '=');
        let k = kv.next()?;
        let v = kv.next()?.trim();
        (k == name && !v.is_empty()).then(|| v.to_string())
    })
}
