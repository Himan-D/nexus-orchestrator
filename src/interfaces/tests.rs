use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

use crate::application::auth::AuthService;
use crate::application::run_manager::RunHub;
use crate::infrastructure::sqlite::SqliteRepo;
use crate::interfaces::middleware::AuthRateLimiter;
use crate::interfaces::{router, AppState};

async fn test_app() -> axum::Router {
    let repo = Arc::new(SqliteRepo::in_memory().await.unwrap());
    repo.run_migrations().await.unwrap();
    let state = AppState {
        jobs: repo.clone(),
        runs: repo.clone(),
        assets: repo.clone(),
        auth: Arc::new(AuthService::new(
            repo.clone(),
            b"test-secret-test-secret-test-secret!",
        )),
        hub: RunHub::new(),
        cookie_secure: false,
        limiter: AuthRateLimiter::new(10_000), // effectively off in tests
    };
    router(state)
}

fn cookie_from(res: &axum::response::Response) -> String {
    res.headers()
        .get(header::SET_COOKIE)
        .expect("session cookie set")
        .to_str()
        .expect("cookie is ascii")
        .to_string()
}

async fn signup(app: &axum::Router, email: &str) -> axum::response::Response {
    let body = Body::from(format!(r#"{{"email":"{email}","password":"password123"}}"#));
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/signup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<String>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
        .await
        .unwrap()
}

const TASKS_JSON: &str = r#""tasks":[{"id":"a","name":"A","command":"true"}]"#;

#[tokio::test]
async fn protected_routes_require_auth() {
    let app = test_app().await;
    let res = send(&app, "GET", "/v1/jobs", None, None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let res = send(&app, "POST", "/v1/jobs", None, Some("{}".into())).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_endpoints_are_public() {
    let app = test_app().await;
    assert_eq!(send(&app, "GET", "/healthz", None, None).await.status(), StatusCode::OK);
    assert_eq!(send(&app, "GET", "/readyz", None, None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn ownership_is_enforced_on_writes() {
    let app = test_app().await;

    // Owner signs up and creates a pipeline.
    let res = signup(&app, "owner@nexus.dev").await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let owner_cookie = cookie_from(&res);

    let res = send(
        &app,
        "POST",
        "/v1/jobs",
        Some(&owner_cookie),
        Some(format!(r#"{{"name":"etl",{TASKS_JSON}}}"#)),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let job: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let job_id = job["id"].as_str().unwrap().to_string();

    // A stranger can read, but not write.
    let res = signup(&app, "stranger@nexus.dev").await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let stranger_cookie = cookie_from(&res);

    let res = send(&app, "GET", "/v1/jobs", Some(&stranger_cookie), None).await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = send(
        &app,
        "PATCH",
        &format!("/v1/jobs/{job_id}"),
        Some(&stranger_cookie),
        Some(r#"{"enabled":false}"#.into()),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = send(
        &app,
        "DELETE",
        &format!("/v1/jobs/{job_id}"),
        Some(&stranger_cookie),
        None,
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = send(
        &app,
        "POST",
        &format!("/v1/jobs/{job_id}/runs"),
        Some(&stranger_cookie),
        None,
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // The owner can still write.
    let res = send(
        &app,
        "PATCH",
        &format!("/v1/jobs/{job_id}"),
        Some(&owner_cookie),
        Some(r#"{"enabled":false}"#.into()),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn invalid_cron_is_rejected() {
    let app = test_app().await;
    let res = signup(&app, "a@nexus.dev").await;
    let cookie = cookie_from(&res);

    let res = send(
        &app,
        "POST",
        "/v1/jobs",
        Some(&cookie),
        Some(format!(r#"{{"name":"bad","schedule":"not a cron",{TASKS_JSON}}}"#)),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = send(
        &app,
        "POST",
        "/v1/jobs",
        Some(&cookie),
        Some(format!(r#"{{"name":"good","schedule":"*/5 * * * *",{TASKS_JSON}}}"#)),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[test]
fn rate_limiter_blocks_after_threshold() {
    let limiter = AuthRateLimiter::new(3);
    let ip: IpAddr = "203.0.113.7".parse().unwrap();
    assert!(limiter.check(ip).is_ok());
    assert!(limiter.check(ip).is_ok());
    assert!(limiter.check(ip).is_ok());
    assert!(limiter.check(ip).is_err());
    // Other IPs are unaffected.
    let other: IpAddr = "198.51.100.9".parse().unwrap();
    assert!(limiter.check(other).is_ok());
}
