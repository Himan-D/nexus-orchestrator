use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::application::ports::{ExecutorError, TaskExecutor, TaskOutput};

/// Maximum raw bytes captured from a process stream before truncation.
const MAX_STREAM_BYTES: usize = 128 * 1024;

/// Runs task commands as local shell processes.
///
/// Security note: this adapter executes arbitrary shell commands **by design**.
/// It must only be used in environments where job definitions are trusted, or
/// behind a sandboxing executor (containers, nsjail, K8s jobs). Swap this type
/// for a sandboxed `TaskExecutor` implementation before accepting pipeline
/// definitions from untrusted users.
pub struct LocalExecutor {
    workdir: Option<std::path::PathBuf>,
}

impl LocalExecutor {
    pub fn new() -> Self {
        Self { workdir: None }
    }

    #[allow(dead_code)]
    pub fn with_workdir(path: impl Into<std::path::PathBuf>) -> Self {
        Self { workdir: Some(path.into()) }
    }
}

impl Default for LocalExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TaskExecutor for LocalExecutor {
    async fn execute(
        &self,
        command: &str,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<TaskOutput, ExecutorError> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // If we abandon the future (timeout/cancel), the dropped child is
            // killed by tokio's kill_on_drop instead of leaking.
            .kill_on_drop(true);
        if let Some(dir) = &self.workdir {
            cmd.current_dir(dir);
        }

        let child_fut = async move {
            let child = cmd.spawn().map_err(|e| ExecutorError::Spawn(e.to_string()))?;
            let out = child
                .wait_with_output()
                .await
                .map_err(|e| ExecutorError::Spawn(e.to_string()))?;
            Ok(TaskOutput {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&tail_bytes(&out.stdout, MAX_STREAM_BYTES))
                    .to_string(),
                stderr: String::from_utf8_lossy(&tail_bytes(&out.stderr, MAX_STREAM_BYTES))
                    .to_string(),
            })
        };

        // Pin and race: if the timeout or cancellation wins, `child_fut` is
        // dropped, which drops the Child, which SIGKILLs it (kill_on_drop).
        // wait_with_output drains both pipes, so no pipe-full deadlock.
        tokio::pin!(child_fut);
        tokio::select! {
            res = &mut child_fut => res,
            _ = tokio::time::sleep(timeout) => Err(ExecutorError::Timeout),
            _ = cancel.cancelled() => Err(ExecutorError::Cancelled),
        }
    }
}

fn tail_bytes(bytes: &[u8], max: usize) -> &[u8] {
    if bytes.len() <= max {
        bytes
    } else {
        &bytes[bytes.len() - max..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_and_captures_output() {
        let ex = LocalExecutor::new();
        let out = ex
            .execute("echo hello; echo oops >&2", Duration::from_secs(10), CancellationToken::new())
            .await
            .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "hello");
        assert_eq!(out.stderr.trim(), "oops");
    }

    #[tokio::test]
    async fn nonzero_exit_reports_failure() {
        let ex = LocalExecutor::new();
        let out = ex
            .execute("exit 3", Duration::from_secs(10), CancellationToken::new())
            .await
            .unwrap();
        assert!(!out.success);
    }

    #[tokio::test]
    async fn timeout_kills_process() {
        let ex = LocalExecutor::new();
        let started = std::time::Instant::now();
        let res = ex
            .execute("sleep 60", Duration::from_millis(300), CancellationToken::new())
            .await;
        assert!(matches!(res, Err(ExecutorError::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancellation_kills_process() {
        let ex = LocalExecutor::new();
        let token = CancellationToken::new();
        let ex2 = ex.execute("sleep 60", Duration::from_secs(60), token.clone());
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            token.cancel();
        });
        let res = ex2.await;
        cancel.await.unwrap();
        assert!(matches!(res, Err(ExecutorError::Cancelled)));
    }
}
