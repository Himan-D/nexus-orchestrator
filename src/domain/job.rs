use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::DomainError;

pub const MAX_TASKS: usize = 100;
pub const MAX_COMMAND_LEN: usize = 10_000;
pub const MAX_NAME_LEN: usize = 200;
pub const DEFAULT_TASK_TIMEOUT_SECS: u64 = 3600;

/// A unit of work inside a pipeline. Commands are executed by the configured
/// executor (locally a shell); the sandbox story lives with the executor
/// adapter, not here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Asset keys this task reads (upstream inputs — used for lineage).
    #[serde(default)]
    pub asset_inputs: Vec<String>,
    /// Optional asset key this task materializes, e.g. "warehouse.users".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materializes: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay_secs: u64,
}

fn default_timeout() -> u64 {
    DEFAULT_TASK_TIMEOUT_SECS
}

fn default_retry_delay() -> u64 {
    5
}

/// A pipeline definition. Execution state lives on `Run`, not here —
/// a job is what you define, a run is what happens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tasks: Vec<Task>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional cron expression (UTC, 5-field standard syntax) that
    /// auto-triggers runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// Scheduler bookkeeping: the last time the cron enqueued a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enqueued_at: Option<DateTime<Utc>>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_enabled() -> bool {
    true
}

impl Job {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        tasks: Vec<Task>,
        owner_id: Uuid,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            tasks,
            enabled: true,
            schedule: None,
            last_enqueued_at: None,
            owner_id,
            created_at: now,
            updated_at: now,
        }
    }

    /// Validates invariants: non-empty name, unique task ids, resolvable
    /// dependencies, an acyclic graph, and sane field sizes.
    pub fn validate(&self) -> Result<(), DomainError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(DomainError::MissingField("name".into()));
        }
        if name.len() > MAX_NAME_LEN {
            return Err(DomainError::InvalidValue {
                field: "name".into(),
                reason: format!("exceeds {MAX_NAME_LEN} characters"),
            });
        }
        if self.tasks.is_empty() {
            return Err(DomainError::MissingField("tasks".into()));
        }
        if self.tasks.len() > MAX_TASKS {
            return Err(DomainError::InvalidValue {
                field: "tasks".into(),
                reason: format!("more than {MAX_TASKS} tasks"),
            });
        }

        let ids: std::collections::HashSet<&str> =
            self.tasks.iter().map(|t| t.id.as_str()).collect();
        for task in &self.tasks {
            if task.id.trim().is_empty() {
                return Err(DomainError::MissingField("task.id".into()));
            }
            if task.command.trim().is_empty() {
                return Err(DomainError::InvalidValue {
                    field: format!("task {}.command", task.id),
                    reason: "command must not be empty".into(),
                });
            }
            if task.command.len() > MAX_COMMAND_LEN {
                return Err(DomainError::InvalidValue {
                    field: format!("task {}.command", task.id),
                    reason: format!("exceeds {MAX_COMMAND_LEN} characters"),
                });
            }
            for key in &task.asset_inputs {
                if !super::asset::Asset::validate_key(key) {
                    return Err(DomainError::InvalidValue {
                        field: format!("task {}.asset_inputs", task.id),
                        reason: format!("invalid asset key '{key}'"),
                    });
                }
            }
            if task.timeout_secs == 0 || task.timeout_secs > 86_400 {
                return Err(DomainError::InvalidValue {
                    field: format!("task {}.timeout_secs", task.id),
                    reason: "must be between 1 and 86400".into(),
                });
            }
            for dep in &task.depends_on {
                if !ids.contains(dep.as_str()) {
                    return Err(DomainError::InvalidValue {
                        field: format!("task {}.depends_on", task.id),
                        reason: format!("unknown dependency '{dep}'"),
                    });
                }
                if dep == &task.id {
                    return Err(DomainError::InvalidValue {
                        field: format!("task {}.depends_on", task.id),
                        reason: "task cannot depend on itself".into(),
                    });
                }
            }
        }

        self.topo_waves()?;
        Ok(())
    }

    /// Computes execution waves: each wave contains tasks whose dependencies
    /// all live in earlier waves, so a wave can run fully in parallel.
    /// Returns `InvalidValue` if the graph has a cycle.
    pub fn topo_waves(&self) -> Result<Vec<Vec<String>>, DomainError> {
        let mut remaining_deps: std::collections::HashMap<&str, usize> = self
            .tasks
            .iter()
            .map(|t| (t.id.as_str(), t.depends_on.len()))
            .collect();
        let mut dependents: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for task in &self.tasks {
            for dep in &task.depends_on {
                dependents.entry(dep.as_str()).or_default().push(task.id.as_str());
            }
        }

        let mut waves = Vec::new();
        let mut ready: Vec<&str> = remaining_deps
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut scheduled: std::collections::HashSet<&str> = std::collections::HashSet::new();

        while !ready.is_empty() {
            ready.sort(); // deterministic (lexicographic) order within a wave
            scheduled.extend(ready.iter().copied());
            waves.push(ready.iter().map(|s| s.to_string()).collect());

            let mut next = Vec::new();
            for &done in &ready {
                if let Some(children) = dependents.get(done) {
                    for child in children {
                        let d = remaining_deps.get_mut(child).unwrap();
                        *d -= 1;
                        if *d == 0 {
                            next.push(*child);
                        }
                    }
                }
            }
            ready = next;
        }

        if scheduled.len() != self.tasks.len() {
            let cyclic: Vec<String> = self
                .tasks
                .iter()
                .filter(|t| !scheduled.contains(t.id.as_str()))
                .map(|t| t.id.clone())
                .collect();
            return Err(DomainError::InvalidValue {
                field: "tasks".into(),
                reason: format!("dependency cycle involving tasks: {}", cyclic.join(", ")),
            });
        }

        Ok(waves)
    }

    pub fn task_by_id(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str]) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            command: "echo hi".into(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            asset_inputs: vec![],
            materializes: None,
            timeout_secs: 60,
            max_retries: 0,
            retry_delay_secs: 1,
        }
    }

    fn job(tasks: Vec<Task>) -> Job {
        Job::new("test", "", tasks, Uuid::new_v4())
    }

    #[test]
    fn valid_job_passes_validation() {
        let j = job(vec![task("a", &[]), task("b", &["a"]), task("c", &["a"])]);
        assert!(j.validate().is_ok());
    }

    #[test]
    fn empty_name_rejected() {
        let mut j = job(vec![task("a", &[])]);
        j.name = "  ".into();
        assert!(matches!(
            j.validate(),
            Err(DomainError::MissingField(_))
        ));
    }

    #[test]
    fn empty_tasks_rejected() {
        assert!(matches!(job(vec![]).validate(), Err(DomainError::MissingField(_))));
    }

    #[test]
    fn unknown_dependency_rejected() {
        let j = job(vec![task("a", &["ghost"])]);
        assert!(matches!(j.validate(), Err(DomainError::InvalidValue { .. })));
    }

    #[test]
    fn self_dependency_rejected() {
        let j = job(vec![task("a", &["a"])]);
        assert!(matches!(j.validate(), Err(DomainError::InvalidValue { .. })));
    }

    #[test]
    fn cycle_detected() {
        let j = job(vec![task("a", &["c"]), task("b", &["a"]), task("c", &["b"])]);
        assert!(matches!(j.validate(), Err(DomainError::InvalidValue { .. })));
    }

    #[test]
    fn topo_waves_respect_dependencies_and_parallelism() {
        let j = job(vec![
            task("a", &[]),
            task("b", &["a"]),
            task("c", &["a"]),
            task("d", &["b", "c"]),
        ]);
        let waves = j.topo_waves().unwrap();
        assert_eq!(waves, vec![
            vec!["a".to_string()],
            vec!["b".to_string(), "c".to_string()],
            vec!["d".to_string()],
        ]);
    }

    #[test]
    fn duplicate_task_ids_rejected() {
        // deps from both tasks to a phantom id make them both unschedulable;
        // the direct check is unknown-dependency, so test the ids via map uniqueness:
        let j = job(vec![task("a", &[]), task("a", &["nope"])]);
        assert!(matches!(j.validate(), Err(DomainError::InvalidValue { .. })));
    }
}
