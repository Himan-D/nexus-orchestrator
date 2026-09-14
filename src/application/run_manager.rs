use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::application::ports::{
    AssetRepository, ExecutorError, JobRepository, RunRepository, TaskExecutor, TaskOutput,
};
use crate::domain::asset::{Asset, Materialization};
use crate::domain::job::Task;
use crate::domain::run::{Run, RunStatus, TaskRun, TaskStatus};

/// Maximum log text retained per task (tail).
const MAX_LOG_BYTES: usize = 64 * 1024;

/// In-memory registry of cancellation tokens for in-flight runs, shared
/// between the API (cancel endpoint) and the run engine.
#[derive(Clone, Default)]
pub struct RunHub {
    tokens: Arc<Mutex<std::collections::HashMap<Uuid, CancellationToken>>>,
}

impl RunHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, run_id: Uuid) -> CancellationToken {
        let token = CancellationToken::new();
        self.tokens.lock().expect("run hub lock").insert(run_id, token.clone());
        token
    }

    pub fn finish(&self, run_id: Uuid) {
        self.tokens.lock().expect("run hub lock").remove(&run_id);
    }

    /// Signals cancellation. Returns false when the run is not in flight
    /// (already finished, or queued and cancellable directly in the DB).
    pub fn cancel(&self, run_id: Uuid) -> bool {
        if let Some(token) = self.tokens.lock().expect("run hub lock").get(&run_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub fn cancel_all(&self) {
        for (_, token) in self.tokens.lock().expect("run hub lock").iter() {
            token.cancel();
        }
    }
}

/// Executes runs: walks the job's DAG in topological waves, runs independent
/// tasks in parallel, applies retries/timeouts, records per-task logs and
/// asset materializations. This is the application-layer orchestration brain.
pub struct RunManager {
    jobs: Arc<dyn JobRepository>,
    runs: Arc<dyn RunRepository>,
    assets: Arc<dyn AssetRepository>,
    executor: Arc<dyn TaskExecutor>,
    hub: RunHub,
    /// Caps how many tasks of a single run execute concurrently, so a
    /// wide DAG cannot fork-bomb the host.
    task_permits: Arc<Semaphore>,
}

struct TaskOutcome {
    failed: bool,
    cancelled: bool,
    error: Option<String>,
}

impl RunManager {
    pub fn new(
        jobs: Arc<dyn JobRepository>,
        runs: Arc<dyn RunRepository>,
        assets: Arc<dyn AssetRepository>,
        executor: Arc<dyn TaskExecutor>,
        hub: RunHub,
        max_parallel_tasks: usize,
    ) -> Self {
        Self {
            jobs,
            runs,
            assets,
            executor,
            hub,
            task_permits: Arc::new(Semaphore::new(max_parallel_tasks.max(1))),
        }
    }

    pub fn hub(&self) -> &RunHub {
        &self.hub
    }

    pub async fn execute_run(&self, run: Run) {
        let token = self.hub.register(run.id);

        if let Err(e) = self.execute_inner(&run, &token).await {
            error!("Run {} crashed: {}", run.id, e);
            let _ = self
                .runs
                .update_status(run.id, RunStatus::Failed, Some(&e.to_string()))
                .await;
        }

        self.hub.finish(run.id);
    }

    async fn execute_inner(&self, run: &Run, token: &CancellationToken) -> anyhow::Result<()> {
        let job = match self.jobs.get(run.job_id).await? {
            Some(j) => j,
            None => {
                // Job was deleted after the run was queued.
                self.finish_run(run, RunStatus::Failed, Some("job no longer exists")).await?;
                return Ok(());
            }
        };

        info!("▶️ Run {} starting for job '{}'", run.id, job.name);

        // Create one TaskRun row per task so the UI can track progress live.
        // Keep the persisted rows — tasks update themselves by id.
        let mut tracked: HashMap<String, TaskRun> = HashMap::new();
        for task in &job.tasks {
            let tr = TaskRun::new(run.id, task);
            self.runs.create_task_run(&tr).await?;
            tracked.insert(task.id.clone(), tr);
        }

        let waves = job.topo_waves().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut first_failure: Option<String> = None;
        let mut was_cancelled = false;

        'waves: for wave in waves {
            if token.is_cancelled() {
                was_cancelled = true;
                break 'waves;
            }

            let mut set = JoinSet::new();
            for task_id in wave {
                let Some(task) = job.task_by_id(&task_id).cloned() else {
                    continue;
                };
                let Some(task_run) = tracked.remove(&task_id) else {
                    continue;
                };
                let ctx = TaskCtx {
                    executor: self.executor.clone(),
                    runs: self.runs.clone(),
                    assets: self.assets.clone(),
                    token: token.clone(),
                    run_id: run.id,
                    permits: self.task_permits.clone(),
                };
                set.spawn(async move { ctx.run_task(task, task_run).await });
            }

            while let Some(joined) = set.join_next().await {
                let outcome = match joined {
                    Ok(o) => o,
                    Err(e) => {
                        // The spawned task itself panicked — treat as failure.
                        first_failure.get_or_insert_with(|| format!("internal task error: {e}"));
                        continue;
                    }
                };
                if outcome.failed {
                    first_failure.get_or_insert_with(|| {
                        outcome.error.clone().unwrap_or_else(|| "task failed".into())
                    });
                }
                if outcome.cancelled {
                    was_cancelled = true;
                }
            }

            // A failed or cancelled wave short-circuits the rest of the DAG;
            // the finalization pass marks everything left as skipped/cancelled.
            if was_cancelled || first_failure.is_some() {
                break 'waves;
            }
        }

        // Finalize tasks that never reached a terminal state. The DB is the
        // source of truth (local copies would be stale).
        let stored = self.runs.task_runs(run.id).await?;
        for mut tr in stored {
            if tr.status.is_terminal() {
                continue;
            }
            let target = if was_cancelled { TaskStatus::Cancelled } else { TaskStatus::Skipped };
            if tr.transition(target).is_ok() {
                tr.finished_at = Some(Utc::now());
                if let Err(e) = self.runs.update_task_run(&tr).await {
                    error!("Failed to persist {} for task {}: {}", target, tr.task_id, e);
                }
            }
        }

        if was_cancelled {
            info!("⏹ Run {} cancelled", run.id);
            self.finish_run(run, RunStatus::Cancelled, Some("cancelled by user")).await?;
        } else if let Some(err) = first_failure {
            warn!("❌ Run {} failed: {}", run.id, err);
            self.finish_run(run, RunStatus::Failed, Some(&err)).await?;
        } else {
            info!("✅ Run {} succeeded", run.id);
            self.finish_run(run, RunStatus::Success, None).await?;
        }
        Ok(())
    }

    async fn finish_run(
        &self,
        run: &Run,
        status: RunStatus,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        // Domain-enforced transition; also guards against double-finishing.
        run.status.transition(status)?;
        self.runs.update_status(run.id, status, error).await?;
        Ok(())
    }
}

/// Everything a single task execution needs, cloned into its tokio task.
struct TaskCtx {
    executor: Arc<dyn TaskExecutor>,
    runs: Arc<dyn RunRepository>,
    assets: Arc<dyn AssetRepository>,
    token: CancellationToken,
    run_id: Uuid,
    permits: Arc<Semaphore>,
}

impl TaskCtx {
    /// Runs one task through its full retry policy. Returns the outcome; the
    /// authoritative state lives in the DB (written on every transition).
    async fn run_task(self, task: Task, mut tr: TaskRun) -> TaskOutcome {
        if self.should_skip(&task).await {
            let _ = tr.transition(TaskStatus::Skipped);
            if let Err(e) = self.runs.update_task_run(&tr).await {
                error!("Failed to persist skipped task {}: {}", tr.task_id, e);
            }
            return TaskOutcome { failed: false, cancelled: false, error: None };
        }

        let max_attempts = task.max_retries.saturating_add(1);
        let mut last_error: Option<String> = None;

        for attempt in 1..=max_attempts {
            if self.token.is_cancelled() {
                let _ = tr.transition(TaskStatus::Cancelled);
                let _ = self.runs.update_task_run(&tr).await;
                return TaskOutcome { failed: false, cancelled: true, error: None };
            }

            let _ = tr.transition(TaskStatus::Running);
            tr.attempts = attempt;
            if tr.started_at.is_none() {
                tr.started_at = Some(Utc::now());
            }
            if let Err(e) = self.runs.update_task_run(&tr).await {
                error!("Failed to persist running state for task {}: {}", tr.task_id, e);
            }

            let started = std::time::Instant::now();
            // Bound the process fan-out per run; cancelled waits don't consume.
            // (The semaphore is never closed, but fail safe rather than panic.)
            let _permit = match self.permits.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    return TaskOutcome {
                        failed: true,
                        cancelled: false,
                        error: Some("internal: task semaphore closed".into()),
                    };
                }
            };
            match self
                .executor
                .execute(&task.command, Duration::from_secs(task.timeout_secs), self.token.clone())
                .await
            {
                Ok(out) if out.success => {
                    tr.logs = Some(combine_logs(&out));
                    let _ = tr.transition(TaskStatus::Success);
                    tr.finished_at = Some(Utc::now());
                    if let Err(e) = self.runs.update_task_run(&tr).await {
                        error!("Failed to persist success for task {}: {}", tr.task_id, e);
                    }
                    self.materialize(&task, &tr).await;
                    info!(
                        "✅ Task '{}' succeeded in {:?} (attempt {}/{})",
                        task.name,
                        started.elapsed(),
                        attempt,
                        max_attempts
                    );
                    return TaskOutcome {
                        failed: false,
                        cancelled: false,
                        error: None,
                    };
                }
                Ok(out) => {
                    tr.logs = Some(combine_logs(&out));
                    last_error = Some(tail_str(&out.stderr, 2000));
                }
                Err(ExecutorError::Timeout) => {
                    last_error = Some(format!("task timed out after {}s", task.timeout_secs));
                }
                Err(ExecutorError::Cancelled) => {
                    let _ = tr.transition(TaskStatus::Cancelled);
                    tr.finished_at = Some(Utc::now());
                    let _ = self.runs.update_task_run(&tr).await;
                    return TaskOutcome { failed: false, cancelled: true, error: None };
                }
                Err(ExecutorError::Spawn(msg)) => {
                    last_error = Some(format!("failed to spawn: {msg}"));
                }
            }

            warn!(
                "⚠️ Task '{}' attempt {}/{} failed: {}",
                task.name,
                attempt,
                max_attempts,
                last_error.clone().unwrap_or_default()
            );

            // Retryable failure: back off (cancellably), then loop.
            if attempt < max_attempts {
                let delay = Duration::from_secs(task.retry_delay_secs.max(1));
                tokio::select! {
                    _ = sleep(delay) => {}
                    _ = self.token.cancelled() => {
                        let _ = tr.transition(TaskStatus::Cancelled);
                        let _ = self.runs.update_task_run(&tr).await;
                        return TaskOutcome { failed: false, cancelled: true, error: None };
                    }
                }
            }
        }

        let _ = tr.transition(TaskStatus::Failed);
        tr.error = last_error.clone();
        tr.finished_at = Some(Utc::now());
        if let Err(e) = self.runs.update_task_run(&tr).await {
            error!("Failed to persist failure for task {}: {}", tr.task_id, e);
        }
        TaskOutcome {
            failed: true,
            cancelled: false,
            error: Some(format!(
                "task '{}' failed after {max_attempts} attempt(s): {}",
                task.name,
                last_error.unwrap_or_else(|| "unknown error".into())
            )),
        }
    }

    /// True when the task must be skipped because an upstream dependency did
    /// not succeed. Fails open (run the task) if statuses cannot be read.
    async fn should_skip(&self, task: &Task) -> bool {
        if task.depends_on.is_empty() {
            return false;
        }
        let all = match self.runs.task_runs(self.run_id).await {
            Ok(list) => list,
            Err(e) => {
                error!("Failed to load dependency statuses: {}", e);
                return false;
            }
        };
        let by_id: std::collections::HashMap<&str, TaskStatus> = all
            .iter()
            .map(|t| (t.task_id.as_str(), t.status))
            .collect();
        task.depends_on.iter().any(|dep| {
            by_id
                .get(dep.as_str())
                .map(|s| !matches!(s, TaskStatus::Success | TaskStatus::Pending | TaskStatus::Running))
                .unwrap_or(false)
        })
    }

    async fn materialize(&self, task: &Task, tr: &TaskRun) {
        let Some(key) = task.materializes.as_ref() else {
            return;
        };
        if !Asset::validate_key(key) {
            warn!("Task '{}' has invalid asset key '{}'; skipping materialization", task.name, key);
            return;
        }
        let asset = match self.assets.get(key).await {
            Ok(Some(existing)) => existing,
            Ok(None) => Asset::new(key.clone()),
            Err(e) => {
                error!("Failed to load asset {key}: {}", e);
                return;
            }
        };
        if let Err(e) = self.assets.upsert(&asset).await {
            error!("Failed to upsert asset {key}: {}", e);
            return;
        }
        let mat = Materialization::new(key.clone(), tr.run_id, tr.id);
        if let Err(e) = self.assets.record_materialization(&mat).await {
            error!("Failed to record materialization for {key}: {}", e);
            return;
        }
        if let Err(e) = self.assets.touch_last_materialized(key, tr.run_id, Utc::now()).await {
            error!("Failed to update freshness for {key}: {}", e);
        }
        info!("📦 Asset '{key}' materialized by task '{}'", task.name);
    }
}

fn combine_logs(out: &TaskOutput) -> String {
    let mut s = String::new();
    if !out.stdout.is_empty() {
        s.push_str(&format!("--- stdout ---\n{}\n", tail_str(&out.stdout, MAX_LOG_BYTES)));
    }
    if !out.stderr.is_empty() {
        s.push_str(&format!("--- stderr ---\n{}", tail_str(&out.stderr, MAX_LOG_BYTES)));
    }
    s.trim().to_string()
}

fn tail_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len() - max_bytes;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_char_boundaries() {
        let s = "ab😀".repeat(20_000);
        let t = tail_str(&s, 1024);
        // "…" is 3 bytes and one char-advance can add up to 3 more.
        assert!(t.len() <= 1024 + 6);
        assert!(t.starts_with('…'));
    }
}
