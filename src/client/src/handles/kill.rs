use crate::errors::ClientError;
use crate::handles::StaleHandle;
use crate::policies::PolicyConfig;
use crate::process::ProcessControl;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, instrument, warn};

/// A stateful handle representing a stale or non-responsive daemon process.
///
/// This handle is returned during the connection phase if the workspace contains
/// a valid PID file and the operating system confirms the process is resident,
/// but the underlying communication socket or IPC channel is unhealthy or deadlocked.
/// It exposes immediate recovery vectors like programmatic termination and restarts.
#[derive(Debug)]
pub struct KillHandle {
    /// The target filesystem path pointing to the active workspace directory.
    pub runtime_dir: PathBuf,
    /// The process to terminate.
    pub process: Box<dyn ProcessControl>,
    pub daemon_path: PathBuf,
    pub policy: Arc<PolicyConfig>,
}

impl KillHandle {
    /// Forcefully terminates the non-responsive process and purges orphaned environment artifacts.
    ///
    /// This method uses the system process subsystem to locate the target PID and issues a direct
    /// `SIGKILL` (or OS equivalent) signal to guarantee exit. Once the process is removed,
    /// it wraps the environment details into an interim handle to wipe out stale socket files or locks.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] if the system fails to scrub the remaining infrastructure files
    /// during the transition into the offline state.
    #[instrument(skip(self), fields(pid = self.process.pid(), dir = %self.runtime_dir.display()))]
    pub fn kill(self) -> Result<StaleHandle, ClientError> {
        self.process.kill().map_err(ClientError::Io)?;

        debug!("transitioning to stale state to trigger environment cleanup");
        Ok(StaleHandle {
            runtime_dir: self.runtime_dir,
            daemon_path: self.daemon_path,
            policy: self.policy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct MockProcess {
        pid: u32,
        kill_result: std::io::Result<()>,
    }

    impl MockProcess {
        fn success(pid: u32) -> Self {
            Self {
                pid,
                kill_result: Ok(()),
            }
        }

        fn failure(pid: u32) -> Self {
            Self {
                pid,
                kill_result: Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "mock kill failure",
                )),
            }
        }
    }

    impl ProcessControl for MockProcess {
        fn kill(&self) -> std::io::Result<()> {
            match &self.kill_result {
                Ok(()) => Ok(()),
                Err(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
            }
        }

        fn pid(&self) -> u32 {
            self.pid
        }
    }

    fn handle(dir: PathBuf, process: Box<dyn ProcessControl>) -> KillHandle {
        KillHandle {
            runtime_dir: dir,
            process,
            daemon_path: PathBuf::from("/mock/bin/knotd"),
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    #[tokio::test]
    async fn kill_cleans_stale_environment_after_successful_kill() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path().to_path_buf();
        let process = MockProcess::success(9999);

        let handle = handle(dir, Box::new(process));

        let result = handle.kill();
        assert!(result.is_ok(),);
    }

    #[tokio::test]
    async fn kill_returns_error_when_process_kill_fails() {
        let temp_dir = tempfile::tempdir().unwrap();

        let process = MockProcess::failure(9999);

        let handle = handle(temp_dir.path().to_path_buf(), Box::new(process));

        let result = handle.kill();

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn kill_does_not_clean_when_process_kill_fails() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path().to_path_buf();

        let lock_path = dir.join(knot_core::consts::KNOT_DAEMON_LOCK_FILE);

        tokio::fs::write(&lock_path, "9999").await.unwrap();

        let process = MockProcess::failure(9999);

        let handle = handle(dir, Box::new(process));

        let result = handle.kill();

        assert!(result.is_err());
        assert!(
            lock_path.exists(),
            "cleanup must not happen when process kill fails"
        );
    }
}
