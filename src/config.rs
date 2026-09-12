use std::env;

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
                s_into_bytes(env::var("ORCH_JWT_SECRET"))
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

        Ok(Self {
            bind_addr: env::var("ORCH_BIND").unwrap_or_else(|_| "0.0.0.0:3002".into()),
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
        })
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn s_into_bytes(s: Result<String, env::VarError>) -> Vec<u8> {
    s.unwrap_or_default().into_bytes()
}
