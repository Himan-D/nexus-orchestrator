use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use super::error::DomainError;

/// Execution lifecycle of a run. Transitions are enforced by `transition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Success,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }

    /// Terminal states cannot transition anywhere.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunStatus::Success | RunStatus::Failed | RunStatus::Cancelled
        )
    }

    pub fn transition(&self, to: RunStatus) -> Result<(), DomainError> {
        let allowed = match self {
            RunStatus::Queued => matches!(to, RunStatus::Running | RunStatus::Cancelled),
            RunStatus::Running => {
                matches!(to, RunStatus::Success | RunStatus::Failed | RunStatus::Cancelled)
            }
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(DomainError::InvalidStateTransition {
                from: self.to_string(),
                to: to.to_string(),
            })
        }
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Uuid,
    pub job_id: Uuid,
    pub job_name: String,
    pub status: RunStatus,
    pub triggered_by: Option<Uuid>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Run {
    pub fn new(job: &super::job::Job, triggered_by: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            job_id: job.id,
            job_name: job.name.clone(),
            status: RunStatus::Queued,
            triggered_by,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }
}

/// Lifecycle of a single task within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Success => "success",
            TaskStatus::Failed => "failed",
            TaskStatus::Skipped => "skipped",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, TaskStatus::Pending | TaskStatus::Running)
    }

    pub fn transition(&self, to: TaskStatus) -> Result<(), DomainError> {
        let allowed = match self {
            TaskStatus::Pending => matches!(
                to,
                TaskStatus::Running | TaskStatus::Skipped | TaskStatus::Cancelled
            ),
            TaskStatus::Running => matches!(
                to,
                TaskStatus::Success | TaskStatus::Failed | TaskStatus::Cancelled
            ),
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(DomainError::InvalidStateTransition {
                from: self.to_string(),
                to: to.to_string(),
            })
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub id: Uuid,
    pub run_id: Uuid,
    pub task_id: String,
    pub name: String,
    pub status: TaskStatus,
    pub attempts: u32,
    pub materializes: Option<String>,
    pub logs: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl TaskRun {
    pub fn new(run_id: Uuid, task: &super::job::Task) -> Self {
        Self {
            id: Uuid::new_v4(),
            run_id,
            task_id: task.id.clone(),
            name: task.name.clone(),
            status: TaskStatus::Pending,
            attempts: 0,
            materializes: task.materializes.clone(),
            logs: None,
            error: None,
            started_at: None,
            finished_at: None,
        }
    }

    pub fn transition(&mut self, to: TaskStatus) -> Result<(), DomainError> {
        self.status.transition(to)?;
        self.status = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_transitions_enforced() {
        let r = RunStatus::Queued;
        assert!(r.transition(RunStatus::Running).is_ok());
        let r = RunStatus::Success;
        // Terminal states reject everything.
        assert!(r.transition(RunStatus::Failed).is_err());
    }

    #[test]
    fn queued_cannot_jump_to_success() {
        assert!(RunStatus::Queued.transition(RunStatus::Success).is_err());
    }

    #[test]
    fn running_can_fail_or_cancel() {
        assert!(RunStatus::Running.transition(RunStatus::Failed).is_ok());
        assert!(RunStatus::Running.transition(RunStatus::Cancelled).is_ok());
        assert!(RunStatus::Running.transition(RunStatus::Queued).is_err());
    }

    #[test]
    fn task_run_transitions_enforced() {
        assert!(TaskStatus::Pending.transition(TaskStatus::Running).is_ok());
        assert!(TaskStatus::Pending.transition(TaskStatus::Skipped).is_ok());
        assert!(TaskStatus::Pending.transition(TaskStatus::Success).is_err());
        assert!(TaskStatus::Running.transition(TaskStatus::Failed).is_ok());
        assert!(TaskStatus::Failed.transition(TaskStatus::Success).is_err());
        assert!(TaskStatus::Success.transition(TaskStatus::Running).is_err());
    }
}
