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
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tokio_util::sync::CancellationToken;

use application::auth::AuthService;
use application::run_manager::{RunHub, RunManager};
use application::scheduler::Scheduler;
use infrastructure::local_executor::LocalExecutor;
use infrastructure::sqlite::SqliteRepo;
use interfaces::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Configuration & observability — fail fast on bad config.
    let cfg = config::Config::from_env()?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    info!("Nexus core starting (dev_mode={})", cfg.dev_mode);

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
    ));

    // 4. Scheduler (starts after recovery; graceful via token).
    let shutdown = CancellationToken::new();
    let scheduler = std::sync::Arc::new(Scheduler::new(
        repo.clone(),
        repo.clone(),
        manager.clone(),
        cfg.scheduler_tick_secs,
        cfg.scheduler_batch,
    ));
    let scheduler_handle = scheduler.clone().start(shutdown.clone());

    // 5. HTTP surface.
    let state = AppState {
        jobs: repo.clone(),
        runs: repo.clone(),
        assets: repo.clone(),
        auth: auth.clone(),
        hub: hub.clone(),
        cookie_secure: cfg.cookie_secure,
    };

    let mut app = interfaces::router(state)
        .layer(TraceLayer::new_for_http())
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

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown.clone()))
        .await?;

    // 6. Drain: cancel in-flight runs and wait for the scheduler to finish.
    shutdown.cancel();
    let _ = tokio::time::timeout(
        Duration::from_secs(cfg.shutdown_grace_secs),
        scheduler_handle,
    )
    .await;
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

/// Extracts the on-disk path from a `sqlite://` URL so we can create its
/// parent directory before SQLite tries to open the file.
fn sqlite_parent_dir(db_url: &str) -> Option<String> {
    let rest = db_url.strip_prefix("sqlite://")?;
    let path = rest.split('?').next()?;
    let path = path.strip_prefix("//").unwrap_or(path);
    let parent = std::path::Path::new(path).parent()?;
    (!parent.as_os_str().is_empty()).then(|| parent.to_string_lossy().to_string())
}
