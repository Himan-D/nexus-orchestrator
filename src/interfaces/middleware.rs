use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
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

/// Fixed-window per-IP rate limiter for the auth endpoints (bcrypt attempts
/// are deliberately expensive; this keeps brute force and DoS at bay).
/// In-process memory only — a multi-node deployment needs a shared limiter.
#[derive(Clone)]
pub struct AuthRateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, (Instant, u32)>>>,
    window: Duration,
    max_per_window: u32,
}

impl AuthRateLimiter {
    pub fn new(max_per_min: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            window: Duration::from_secs(60),
            max_per_window: max_per_min.max(1),
        }
    }

    pub fn check(&self, ip: IpAddr) -> Result<(), ApiError> {
        let mut map = self.inner.lock().expect("rate limiter lock");
        let now = Instant::now();
        let entry = map.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        if entry.1 > self.max_per_window {
            // Opportunistic cleanup so the map cannot grow unboundedly.
            map.retain(|_, (start, _)| now.duration_since(*start) < self.window);
            return Err(ApiError::too_many_requests(
                "too many auth attempts; try again in a minute",
            ));
        }
        Ok(())
    }
}

/// Middleware wrapper for the rate limiter. Keyed by client IP when the
/// server is built with `into_make_service_with_connect_info`; otherwise
/// (e.g. in unit tests) the request passes through.
pub async fn auth_rate_limit(
    State(limiter): State<AuthRateLimiter>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        limiter.check(addr.ip())?;
    }
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
