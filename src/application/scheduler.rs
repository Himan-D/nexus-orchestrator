use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::application::ports::{JobRepository, RunRepository};
use crate::application::run_manager::RunManager;
use crate::application::schedules;

/// Polls the repository for queued runs and dispatches them to the run
/// engine, and enqueues due cron-scheduled pipelines. Claims are atomic, so
/// multiple scheduler processes can share a database without double-
/// dispatching; in-flight runs are capped so the host cannot be flooded.
pub struct Scheduler {
    runs: Arc<dyn RunRepository>,
    jobs: Arc<dyn JobRepository>,
    manager: Arc<RunManager>,
    tick: Duration,
    batch: usize,
    max_concurrent_runs: usize,
}

impl Scheduler {
    pub fn new(
        runs: Arc<dyn RunRepository>,
        jobs: Arc<dyn JobRepository>,
        manager: Arc<RunManager>,
        tick_secs: u64,
        batch: usize,
        max_concurrent_runs: usize,
    ) -> Self {
        Self {
            runs,
            jobs,
            manager,
            tick: Duration::from_secs(tick_secs.max(1)),
            batch: batch.max(1),
            max_concurrent_runs: max_concurrent_runs.max(1),
        }
    }

    /// Starts the scheduler loop. Recovers runs orphaned by a previous
    /// process, enqueues due schedules, claims and dispatches queued runs
    /// until `shutdown` fires. On shutdown, in-flight runs are asked to
    /// cancel and we wait for them.
    pub fn start(self: Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.recover_orphans().await;
            info!(
                "⚙️ Scheduler started (tick {:?}, batch {}, max concurrent runs {})",
                self.tick, self.batch, self.max_concurrent_runs
            );

            let mut in_flight: JoinSet<()> = JoinSet::new();

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = sleep(self.tick) => {
                        self.enqueue_due_schedules().await;

                        // Never claim more than the concurrency budget allows.
                        let slots = self
                            .max_concurrent_runs
                            .saturating_sub(in_flight.len());
                        if slots == 0 {
                            continue;
                        }

                        match self.runs.claim_queued(self.batch.min(slots)).await {
                            Ok(claimed) if claimed.is_empty() => {}
                            Ok(claimed) => {
                                for run in claimed {
                                    info!("🚀 Dispatching run {} for job '{}'", run.id, run.job_name);
                                    let mgr = self.manager.clone();
                                    in_flight.spawn(async move { mgr.execute_run(run).await });
                                }
                            }
                            Err(e) => warn!("Scheduler claim failed: {}", e),
                        }
                    }
                }
            }

            // Graceful shutdown: cancel every in-flight run and wait.
            info!("⚙️ Scheduler shutting down; cancelling in-flight runs...");
            self.manager.hub().cancel_all();
            while in_flight.join_next().await.is_some() {}
            info!("⚙️ Scheduler stopped.");
        })
    }

    /// Enqueues runs for cron-scheduled pipelines whose fire time has passed
    /// since the last enqueue. Returns how many runs were enqueued.
    pub async fn enqueue_due_schedules(&self) -> usize {
        let scheduled = match self.jobs.list_scheduled().await {
            Ok(jobs) => jobs,
            Err(e) => {
                warn!("Scheduler failed to load scheduled jobs: {}", e);
                return 0;
            }
        };

        let now = Utc::now();
        let mut enqueued = 0;
        for mut job in scheduled {
            let schedule = match schedules::parse(job.schedule.as_deref().unwrap_or_default()) {
                Ok(s) => s,
                Err(e) => {
                    // A pipeline was saved with a schedule the parser accepts
                    // at the API layer; if it fails here, skip and log rather
                    // than wedging the loop.
                    error!("Job '{}' has unparsable schedule: {}", job.name, e);
                    continue;
                }
            };
            if !schedules::is_due(&schedule, now, job.last_enqueued_at) {
                continue;
            }

            let run = crate::domain::run::Run::new(&job, None);
            if let Err(e) = self.runs.create(&run).await {
                error!("Failed to enqueue scheduled run for '{}': {}", job.name, e);
                continue;
            }
            job.last_enqueued_at = Some(now);
            if let Err(e) = self.jobs.update(&job).await {
                error!("Failed to record enqueue time for '{}': {}", job.name, e);
            }
            info!("⏰ Schedule '{}' enqueued run {} for job '{}'", job.schedule.as_deref().unwrap_or(""), run.id, job.name);
            enqueued += 1;
        }
        enqueued
    }

    async fn recover_orphans(&self) {
        match self.runs.fail_interrupted().await {
            Ok(0) => {}
            Ok(n) => warn!("♻️ Recovered {n} run(s) orphaned by a previous process (marked failed)"),
            Err(e) => error!("Orphan recovery failed: {}", e),
        }
        let _ = &self.jobs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::AssetRepository;
    use crate::application::run_manager::RunHub;
    use crate::domain::job::{Job, Task};
    use crate::domain::run::Run;
    use crate::infrastructure::local_executor::LocalExecutor;
    use crate::infrastructure::sqlite::SqliteRepo;
    use std::sync::Arc;
    use uuid::Uuid;

    async fn setup() -> (
        Arc<dyn JobRepository>,
        Arc<dyn RunRepository>,
        Arc<dyn AssetRepository>,
        Arc<RunManager>,
    ) {
        let repo = Arc::new(SqliteRepo::in_memory().await.unwrap());
        repo.run_migrations().await.unwrap();
        let jobs: Arc<dyn JobRepository> = repo.clone();
        let runs: Arc<dyn RunRepository> = repo.clone();
        let assets: Arc<dyn AssetRepository> = repo.clone();
        let manager = Arc::new(RunManager::new(
            jobs.clone(),
            runs.clone(),
            assets.clone(),
            Arc::new(LocalExecutor::new()),
            RunHub::new(),
            8,
        ));
        (jobs, runs, assets, manager)
    }

    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            command: "echo hello".into(),
            depends_on: vec![],
            asset_inputs: vec![],
            materializes: Some("test.asset".into()),
            timeout_secs: 30,
            max_retries: 0,
            retry_delay_secs: 1,
        }
    }

    fn sample_job(owner: Uuid) -> Job {
        Job::new("demo", "", vec![task("a")], owner)
    }

    #[tokio::test]
    async fn end_to_end_run_executes_and_materializes() {
        let (jobs, runs, assets, manager) = setup().await;
        let job = sample_job(Uuid::new_v4());
        jobs.create(&job).await.unwrap();

        let run = Run::new(&job, None);
        runs.create(&run).await.unwrap();

        let claimed = runs.claim_queued(10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        // Claiming again must not double-dispatch.
        assert_eq!(runs.claim_queued(10).await.unwrap().len(), 0);

        manager.execute_run(claimed.into_iter().next().unwrap()).await;

        let finished = runs.get(run.id).await.unwrap().unwrap();
        assert_eq!(finished.status, crate::domain::run::RunStatus::Success);
        assert!(finished.finished_at.is_some());

        let trs = runs.task_runs(run.id).await.unwrap();
        assert_eq!(trs.len(), 1);
        assert_eq!(trs[0].status, crate::domain::run::TaskStatus::Success);
        assert!(trs[0].logs.as_deref().unwrap_or_default().contains("hello"));

        // The asset was materialized with freshness.
        let asset = assets.get("test.asset").await.unwrap().unwrap();
        assert_eq!(asset.last_run_id, Some(run.id));
        let mats = assets.materializations("test.asset", 10).await.unwrap();
        assert_eq!(mats.len(), 1);
    }

    #[tokio::test]
    async fn failing_task_fails_run_and_cascades() {
        let (jobs, runs, _assets, manager) = setup().await;
        let mut boom = task("boom");
        boom.name = "Boom".into();
        boom.command = "exit 1".into();
        boom.max_retries = 1;
        let mut after = task("after");
        after.command = "echo never".into();
        after.depends_on = vec!["boom".into()];
        after.materializes = None;

        let job = Job::new("failing", "", vec![boom, after], Uuid::new_v4());
        jobs.create(&job).await.unwrap();
        let run = Run::new(&job, None);
        runs.create(&run).await.unwrap();

        let claimed = runs.claim_queued(10).await.unwrap();
        manager.execute_run(claimed.into_iter().next().unwrap()).await;

        let finished = runs.get(run.id).await.unwrap().unwrap();
        assert_eq!(finished.status, crate::domain::run::RunStatus::Failed);
        assert!(finished.error.as_deref().unwrap_or_default().contains("Boom"));

        let trs = runs.task_runs(run.id).await.unwrap();
        let after = trs.iter().find(|t| t.task_id == "after").unwrap();
        assert_eq!(after.status, crate::domain::run::TaskStatus::Skipped);
    }

    #[tokio::test]
    async fn orphan_recovery_marks_running_failed() {
        let (jobs, runs, _assets, _manager) = setup().await;
        let job = sample_job(Uuid::new_v4());
        jobs.create(&job).await.unwrap();
        let mut run = Run::new(&job, None);
        run.status = crate::domain::run::RunStatus::Running;
        runs.create(&run).await.unwrap();

        let n = runs.fail_interrupted().await.unwrap();
        assert_eq!(n, 1);
        let recovered = runs.get(run.id).await.unwrap().unwrap();
        assert_eq!(recovered.status, crate::domain::run::RunStatus::Failed);
    }

    #[tokio::test]
    async fn cron_schedule_enqueues_and_records_enqueue_time() {
        let (jobs, runs, _assets, manager) = setup().await;
        let mut job = sample_job(Uuid::new_v4());
        job.schedule = Some("* * * * *".into());
        // A brand-new schedule waits for the next fire time; a schedule whose
        // fire time has already passed (worker was down) catches up now.
        job.last_enqueued_at = Some(Utc::now() - chrono::Duration::seconds(90));
        jobs.create(&job).await.unwrap();

        let scheduler = Scheduler::new(runs.clone(), jobs.clone(), manager, 1, 5, 4);

        let n = scheduler.enqueue_due_schedules().await;
        assert_eq!(n, 1);
        // Already enqueued within this minute → second pass is not due.
        assert_eq!(scheduler.enqueue_due_schedules().await, 0);

        // The queued run exists and is claimable.
        assert_eq!(runs.claim_queued(10).await.unwrap().len(), 1);
        let updated = jobs.get(job.id).await.unwrap().unwrap();
        assert!(updated.last_enqueued_at.is_some());
    }

    #[tokio::test]
    async fn fresh_schedule_waits_for_next_fire_time() {
        let (jobs, runs, _assets, manager) = setup().await;
        let mut job = sample_job(Uuid::new_v4());
        job.schedule = Some("* * * * *".into());
        jobs.create(&job).await.unwrap();

        let scheduler = Scheduler::new(runs.clone(), jobs.clone(), manager, 1, 5, 4);
        // No history: the next cron tick (a minute away) enqueues, not now.
        assert_eq!(scheduler.enqueue_due_schedules().await, 0);
        assert!(runs.list(None, 10, 0).await.unwrap().is_empty());
    }
}
