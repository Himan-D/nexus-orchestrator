use std::env;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Serve the HTTP API only (no run execution).
    Api,
    /// Execute runs only (no HTTP server).
    Worker,
    /// Single-process mode: API + scheduler (default, good for dev).
    All,
}

impl Role {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_lowercase().as_str() {
            "api" => Ok(Role::Api),
            "worker" => Ok(Role::Worker),
            "all" => Ok(Role::All),
            other => anyhow::bail!("unknown role '{other}'; expected api | worker | all"),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Role::Api => "api",
            Role::Worker => "worker",
            Role::All => "all",
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub db_url: String,
    pub jwt_secret: Vec<u8>,
    pub cookie_secure: bool,
    pub cors_origin: Option<String>,
    pub scheduler_tick_secs: u64,
    pub scheduler_batch: usize,
    pub shutdown_grace_secs: u64,
    pub dev_mode: bool,
    pub role: Role,
    /// Maximum runs executing at the same time (scheduler claim cap).
    pub max_concurrent_runs: usize,
    /// Maximum tasks of one run executing in parallel (per-run semaphore).
    pub max_parallel_tasks: usize,
    /// Auth endpoint attempts allowed per source IP per minute.
    pub auth_rate_limit_per_min: u32,
    /// Per-request timeout applied to the HTTP API.
    pub request_timeout_secs: u64,
}

impl Config {
    /// Loads configuration from the environment. Fails fast on invalid values:
    /// a misconfigured production deployment should not boot.
    pub fn from_env() -> anyhow::Result<Self> {
        let dev_mode = env_flag("ORCH_DEV_MODE");

        let jwt_secret = match env::var("ORCH_JWT_SECRET") {
            Ok(s) if s.len() >= 32 => s.into_bytes(),
            Ok(_) if dev_mode => {
                tracing::warn!("ORCH_JWT_SECRET shorter than 32 chars; using it anyway (dev mode)");
                env::var("ORCH_JWT_SECRET").unwrap_or_default().into_bytes()
            }
            Ok(_) => anyhow::bail!(
                "ORCH_JWT_SECRET must be at least 32 characters. \
                 Generate one with: openssl rand -hex 32"
            ),
            Err(_) if dev_mode => {
                tracing::warn!("ORCH_JWT_SECRET not set; generating an ephemeral secret (dev mode). \
                                Tokens will be invalidated on restart.");
                uuid::Uuid::new_v4().to_string().into_bytes()
            }
            Err(_) => anyhow::bail!("ORCH_JWT_SECRET is required (or set ORCH_DEV_MODE=1 for local dev)"),
        };

        // Dev mode must never bind a LAN-facing interface by accident.
        let default_bind = if dev_mode { "127.0.0.1:3002" } else { "0.0.0.0:3002" };

        Ok(Self {
            bind_addr: env::var("ORCH_BIND").unwrap_or_else(|_| default_bind.into()),
            db_url: env::var("ORCH_DB_URL")
                .unwrap_or_else(|_| "sqlite://data/orchestrator.db?mode=rwc".into()),
            jwt_secret,
            cookie_secure: env::var("ORCH_COOKIE_SECURE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            cors_origin: env::var("ORCH_CORS_ORIGIN").ok(),
            scheduler_tick_secs: env::var("ORCH_SCHED_TICK_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            scheduler_batch: env::var("ORCH_SCHED_BATCH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            shutdown_grace_secs: env::var("ORCH_SHUTDOWN_GRACE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            dev_mode,
            role: match env::var("ORCH_ROLE") {
                Ok(r) => Role::parse(&r)?,
                Err(_) => Role::All,
            },
            max_concurrent_runs: env::var("ORCH_MAX_CONCURRENT_RUNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(4),
            max_parallel_tasks: env::var("ORCH_MAX_PARALLEL_TASKS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(8),
            auth_rate_limit_per_min: env::var("ORCH_AUTH_RATE_LIMIT_PER_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(30),
            request_timeout_secs: env::var("ORCH_REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(30),
        })
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
