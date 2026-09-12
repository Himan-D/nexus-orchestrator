use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A data asset is the unit of value a pipeline produces (a table, a file,
/// a model). Assets — not tasks — are what teams reason about; runs are the
/// history of how each asset came to be.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    /// Stable identifier, e.g. "warehouse.users" or "s3://lake/events".
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub last_run_id: Option<Uuid>,
    pub last_materialized_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Asset {
    pub fn new(key: impl Into<String>) -> Self {
        let key = key.into();
        let name = key.rsplit(['.', '/']).next().unwrap_or(&key).to_string();
        Self {
            key,
            name,
            description: String::new(),
            last_run_id: None,
            last_materialized_at: None,
            created_at: Utc::now(),
        }
    }

    pub fn validate_key(key: &str) -> bool {
        !key.trim().is_empty() && key.len() <= 300 && key.chars().all(|c| c.is_alphanumeric() || "._:/-".contains(c))
    }
}

/// A point-in-time record that an asset was produced by a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Materialization {
    pub id: Uuid,
    pub asset_key: String,
    pub run_id: Uuid,
    pub task_run_id: Uuid,
    pub created_at: DateTime<Utc>,
}

impl Materialization {
    pub fn new(asset_key: impl Into<String>, run_id: Uuid, task_run_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            asset_key: asset_key.into(),
            run_id,
            task_run_id,
            created_at: Utc::now(),
        }
    }
}
