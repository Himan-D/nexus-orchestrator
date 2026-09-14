mod application;
mod config;
mod domain;
mod infrastructure;
mod interfaces;

use std::time::Duration;

use anyhow::Context;
use axum::extract::DefaultBodyLimit;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use tokio_util::sync::CancellationToken;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

use application::auth::AuthService;
use application::run_manager::{RunHub, RunManager};
use application::scheduler::Scheduler;
use config::Role;
use infrastructure::local_executor::LocalExecutor;
use infrastructure::sqlite::SqliteRepo;
use interfaces::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Configuration & observability — fail fast on bad config.
    let mut cfg = config::Config::from_env()?;
    apply_cli_overrides(&mut cfg);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    info!("Nexus core starting (role={}, dev_mode={})", cfg.role, cfg.dev_mode);

    // `--probe` mode: used by container healthchecks (the runtime image
    // intentionally ships without curl/wget).
    if probe_requested() {
        return probe(&cfg.bind_addr);
    }

    // 2. Persistence: make sure the SQLite file's directory exists, connect,
    //    and apply versioned migrations.
    if let Some(parent) = sqlite_parent_dir(&cfg.db_url) {
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("creating database directory {parent}"))?;
    }
    let repo = std::sync::Arc::new(SqliteRepo::connect(&cfg.db_url).await?);
    repo.run_migrations().await?;
    info!("Database ready at {}", cfg.db_url);

    // 3. Application services.
    let auth = std::sync::Arc::new(AuthService::new(repo.clone(), &cfg.jwt_secret));
    let hub = RunHub::new();
    let manager = std::sync::Arc::new(RunManager::new(
        repo.clone(),
        repo.clone(),
        repo.clone(),
        std::sync::Arc::new(LocalExecutor::new()),
        hub.clone(),
        cfg.max_parallel_tasks,
    ));

    let shutdown = CancellationToken::new();
    let mut scheduler_handle: Option<tokio::task::JoinHandle<()>> = None;

    // 4. Scheduler — owned by the worker (or the single-process role).
    if matches!(cfg.role, Role::Worker | Role::All) {
        let scheduler = std::sync::Arc::new(Scheduler::new(
            repo.clone(),
            repo.clone(),
            manager.clone(),
            cfg.scheduler_tick_secs,
            cfg.scheduler_batch,
            cfg.max_concurrent_runs,
        ));
        scheduler_handle = Some(scheduler.clone().start(shutdown.clone()));
    } else {
        info!("API-only role: run execution is delegated to workers");
    }

    // 5. HTTP surface — served by the api (or single-process) role.
    if matches!(cfg.role, Role::Api | Role::All) {
        let state = AppState {
            jobs: repo.clone(),
            runs: repo.clone(),
            assets: repo.clone(),
            auth: auth.clone(),
            hub: hub.clone(),
            cookie_secure: cfg.cookie_secure,
            limiter: interfaces::middleware::AuthRateLimiter::new(cfg.auth_rate_limit_per_min),
        };

        let mut app = interfaces::router(state)
            .layer(TraceLayer::new_for_http())
            .layer(TimeoutLayer::new(Duration::from_secs(cfg.request_timeout_secs)))
            .layer(DefaultBodyLimit::max(1_048_576));

        if let Some(origin) = &cfg.cors_origin {
            let origin_value: HeaderValue = origin
                .parse()
                .with_context(|| format!("invalid ORCH_CORS_ORIGIN '{origin}'"))?;
            app = app.layer(
                tower_http::cors::CorsLayer::new()
                    .allow_origin(origin_value)
                    .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                    .allow_headers([AUTHORIZATION, CONTENT_TYPE])
                    .allow_credentials(true),
            );
            info!("CORS enabled for origin {origin:?}");
        }

        let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
        info!("Nexus core listening on http://{}", cfg.bind_addr);

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal(shutdown.clone()))
        .await?;
    } else {
        // Worker-only: run until signalled.
        shutdown_signal(shutdown.clone()).await;
    }

    // 6. Drain: cancel in-flight runs and wait for the scheduler to finish.
    shutdown.cancel();
    if let Some(handle) = scheduler_handle {
        let _ = tokio::time::timeout(
            Duration::from_secs(cfg.shutdown_grace_secs),
            handle,
        )
        .await;
    }
    info!("Shutdown complete.");
    Ok(())
}

/// Cancels the shutdown token on SIGINT/SIGTERM; axum stops accepting
/// connections and in-flight requests finish first.
async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutdown signal received.");
    token.cancel();
}

fn probe_requested() -> bool {
    std::env::args().any(|a| a == "--probe")
}

fn apply_cli_overrides(cfg: &mut config::Config) {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--role") {
        if let Some(role) = args.get(pos + 1) {
            if let Ok(r) = Role::parse(role) {
                cfg.role = r;
            }
        }
    }
}

/// Healthcheck: the process's HTTP listener accepts connections. Binding on
/// a wildcard is probed through loopback.
fn probe(bind_addr: &str) -> anyhow::Result<()> {
    let mut addr: std::net::SocketAddr = bind_addr
        .parse()
        .or_else(|_| anyhow::bail!("invalid ORCH_BIND '{bind_addr}' for probe"))?;
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(_) => {
            info!("probe ok: {addr} accepting connections");
            Ok(())
        }
        Err(e) => anyhow::bail!("probe failed: {e}"),
    }
}

/// Extracts the on-disk path from a `sqlite://` URL so we can create its
/// parent directory before SQLite tries to open the file.
fn sqlite_parent_dir(db_url: &str) -> Option<String> {
    let rest = db_url.strip_prefix("sqlite://")?;
    let path = rest.split('?').next()?;
    let path = path.strip_prefix("//").unwrap_or(path);
    let parent = std::path::Path::new(path).parent()?;
    (!parent.as_os_str().is_empty()).then(|| parent.to_string_lossy().to_string())
}
