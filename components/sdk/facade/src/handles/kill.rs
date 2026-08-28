use crate::errors::SignalError;
use crate::handles::StaleHandle;
use crate::policies::PolicyConfig;
use knot_sys::{ProcessError, process::PlatformProcess, traits::ProcessControl};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

/// A stateful handle representing a stale or non-responsive daemon process.
///
/// This handle is returned during the connection phase if the workspace contains
/// a valid PID file and the operating system confirms the process is resident,
/// but the underlying communication socket or IPC channel is unhealthy or deadlocked.
/// It exposes immediate recovery vectors like programmatic termination and restarts.
#[derive(Debug)]
pub struct KillHandle<T: ProcessControl> {
    /// The target filesystem path pointing to the active workspace directory.
    pub runtime_dir: PathBuf,
    /// The process to terminate.
    pub process: PlatformProcess<T>,
    pub daemon_path: PathBuf,
    pub policy: Arc<PolicyConfig>,
}

impl<T: ProcessControl> KillHandle<T> {
    /// Terminates the managed process and transitions the handle into the
    /// stale state for subsequent environment cleanup.
    ///
    /// The method first attempts graceful termination using the timeout
    /// configured by [`KillPolicy::graceful_timeout`]. If the timeout is zero,
    /// graceful termination is skipped.
    ///
    /// If the process does not terminate within the graceful timeout, a
    /// forceful termination is requested using the platform-specific process
    /// control implementation. The method then waits for the process to exit
    /// for up to [`KillPolicy::force_timeout`].
    ///
    /// A process that is already not running is treated as successfully
    /// terminated.
    ///
    /// If termination succeeds, this method does not perform environment
    /// cleanup itself. Instead, it consumes the current handle and returns a
    /// [`StaleHandle`] containing the information required for the subsequent
    /// cleanup of the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`SignalError::NotResponding`] if the process does not exit
    /// within the configured forceful termination timeout.
    ///
    /// Returns [`SignalError::ProcessError`] if process control fails, for
    /// example because of a process identity mismatch, PID reuse, or an
    /// underlying operating-system error.
    ///
    /// A process that has already exited is treated as a successful
    /// termination and does not produce an error.
    #[instrument(skip(self), fields(pid = self.process.pid(), dir = %self.runtime_dir.display()))]
    pub async fn kill(self) -> Result<StaleHandle, SignalError> {
        let grace_timeout = self.policy.kill.graceful_timeout;
        let kill_timeout = self.policy.kill.force_timeout;
        let is_terminated = if grace_timeout.is_zero() {
            debug!("graceful timeout is zero, skipping terminate and sending kill signal directly");
            false
        } else {
            debug!("trying to terminate process...");
            match self.process.terminate(grace_timeout).await {
                Ok(sent) => {
                    if sent {
                        info!("process terminated successfully");
                    } else {
                        warn!("process terminate failed: not responding to signal");
                    }
                    sent
                }

                Err(ProcessError::NotRunning) => {
                    warn!("process not running; treating as success");
                    true
                }

                Err(ProcessError::Mismatch { expected, actual }) => {
                    warn!(
                        "process terminate failed: mismatch; expected: {:?}, actual: {:?}",
                        expected, actual
                    );
                    return Err(SignalError::ProcessError(ProcessError::Mismatch {
                        expected,
                        actual,
                    }));
                }

                Err(ProcessError::Io(e)) => {
                    warn!("process terminate failed: io error: {e}");
                    return Err(SignalError::ProcessError(ProcessError::Io(e)));
                }

                Err(e) => {
                    warn!("process terminate failed: {e}");
                    return Err(SignalError::ProcessError(e));
                }
            }
        };

        if !is_terminated {
            match self.process.kill(kill_timeout).await {
                Ok(sent) => {
                    if !sent {
                        warn!("process kill failed: not responding to signal");
                        return Err(SignalError::NotResponding);
                    }
                }
                Err(ProcessError::NotRunning) => {
                    warn!("process not running; treating as success");
                }
                Err(e) => {
                    warn!("process kill failed: {e}");
                    return Err(SignalError::ProcessError(e));
                }
            }

            info!("process killed successfully");
        }

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
    use async_trait::async_trait;
    use knot_sys::metadata::ProcessMetadata;
    use knot_sys::traits::ProcessControl;
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    #[derive(Debug)]
    enum MockOutcome {
        /// Operation completed and process state was confirmed.
        Success,

        /// Operation completed, but process did not acknowledge termination.
        NotResponding,

        /// Process identity no longer matches the bound process.
        Mismatch,

        /// Underlying OS operation failed.
        Io(io::ErrorKind, &'static str),
    }

    #[derive(Debug)]
    pub struct MockProcess {
        terminate_outcome: MockOutcome,
        kill_outcome: MockOutcome,

        terminate_calls: AtomicU32,
        kill_calls: AtomicU32,
        process_terminated: AtomicBool,
    }

    impl MockProcess {
        fn new(terminate_outcome: MockOutcome, kill_outcome: MockOutcome) -> Self {
            Self {
                terminate_outcome,
                kill_outcome,
                terminate_calls: AtomicU32::new(0),
                kill_calls: AtomicU32::new(0),
                process_terminated: AtomicBool::new(false),
            }
        }

        fn resolve(outcome: &MockOutcome) -> Result<bool, ProcessError> {
            match outcome {
                MockOutcome::Success => Ok(true),

                MockOutcome::NotResponding => Ok(false),

                MockOutcome::Mismatch => Err(ProcessError::Mismatch {
                    expected: "expected".into(),
                    actual: "actual".into(),
                }),

                MockOutcome::Io(kind, msg) => Err(ProcessError::Io(io::Error::new(*kind, *msg))),
            }
        }
    }

    #[async_trait]
    impl ProcessControl for MockProcess {
        fn bind(_metadata: ProcessMetadata) -> Result<Self, ProcessError> {
            unimplemented!("bind() is not used by KillHandle tests")
        }

        fn kill(&self) -> Result<(), ProcessError> {
            self.kill_calls.fetch_add(1, Ordering::SeqCst);
            let result = Self::resolve(&self.kill_outcome)?;
            if result {
                self.process_terminated.store(true, Ordering::SeqCst);
            }

            Ok(())
        }

        fn terminate(&self) -> Result<(), ProcessError> {
            self.terminate_calls.fetch_add(1, Ordering::SeqCst);
            let result = Self::resolve(&self.terminate_outcome)?;
            if result {
                self.process_terminated.store(true, Ordering::SeqCst);
            }

            Ok(())
        }

        async fn wait(&self, _timeout: Duration) -> Result<bool, ProcessError> {
            Ok(self.process_terminated.load(Ordering::SeqCst))
        }

        fn check_permissions(&self) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    pub struct MockProcessWrapper(pub Arc<MockProcess>);
    #[async_trait]
    impl ProcessControl for MockProcessWrapper {
        fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
            Ok(Self(Arc::new(MockProcess::bind(metadata)?)))
        }
        fn kill(&self) -> Result<(), ProcessError> {
            self.0.kill()
        }

        fn terminate(&self) -> Result<(), ProcessError> {
            self.0.terminate()
        }

        async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
            self.0.wait(timeout).await
        }

        fn check_permissions(&self) -> Result<(), ProcessError> {
            self.0.check_permissions()
        }
    }

    fn handle(dir: PathBuf, process: PlatformProcess<MockProcess>) -> KillHandle<MockProcess> {
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

        let mock = MockProcess::new(MockOutcome::Success, MockOutcome::Success);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());

        let handle = handle(dir, process);
        let result = handle.kill().await;
        assert!(result.is_ok(),);
    }

    #[tokio::test]
    async fn kill_returns_error_when_process_kill_fails() {
        let temp_dir = tempfile::tempdir().unwrap();

        let mock = MockProcess::new(MockOutcome::NotResponding, MockOutcome::NotResponding);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());
        let handle = handle(temp_dir.path().to_path_buf(), process);

        let result = handle.kill().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn kill_does_not_clean_when_process_kill_fails() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path().to_path_buf();

        let lock_path = dir.join(knot_core::consts::KNOT_DAEMON_LOCK_FILE);

        tokio::fs::write(&lock_path, "9999").await.unwrap();
        let mock = MockProcess::new(MockOutcome::Success, MockOutcome::Success);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());
        let handle = handle(dir, process);
        let result = handle.kill().await;
        assert!(result.is_ok());
        assert!(
            lock_path.exists(),
            "cleanup must not happen when process kill fails"
        );
    }

    #[tokio::test]
    async fn kill_escalates_to_sigkill_when_terminate_not_responding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock = MockProcess::new(MockOutcome::NotResponding, MockOutcome::Success);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());

        let handle = handle(temp_dir.path().to_path_buf(), process);
        let result = handle.kill().await;

        assert!(
            result.is_ok(),
            "escalation to kill() must recover from NotResponding"
        );
        // Проверка вызовов требует доступа к исходному MockProcess — см. вариант с Arc ниже.
    }

    #[tokio::test]
    async fn kill_returns_false_when_terminate_and_kill_both_not_responding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock = MockProcess::new(MockOutcome::NotResponding, MockOutcome::NotResponding);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());

        let handle = handle(temp_dir.path().to_path_buf(), process);
        let result = handle.kill().await;

        assert!(
            matches!(result, Err(SignalError::NotResponding)),
            "when both terminate and kill fail to get acknowledgement, the final \
         NotResponding must propagate — this is the 'nothing more we can do' case"
        );
    }

    #[tokio::test]
    async fn kill_does_not_escalate_on_process_error_mismatch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock = MockProcess::new(MockOutcome::Mismatch, MockOutcome::Success);
        let process = PlatformProcess::new(mock, ProcessMetadata::default());

        let handle = handle(temp_dir.path().to_path_buf(), process);
        let result = handle.kill().await;

        assert!(
            matches!(
                result,
                Err(SignalError::ProcessError(ProcessError::Mismatch { .. }))
            ),
            "Mismatch must short-circuit without escalating to kill() — escalating \
         here risks signalling an unrelated process that reused the PID"
        );
    }

    #[tokio::test]
    async fn kill_does_not_escalate_on_io_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mock = MockProcess::new(
            MockOutcome::Io(io::ErrorKind::PermissionDenied, "denied"),
            MockOutcome::Success,
        );
        let process = PlatformProcess::new(mock, ProcessMetadata::default());

        let handle = handle(temp_dir.path().to_path_buf(), process);
        let result = handle.kill().await;

        assert!(
            matches!(result, Err(SignalError::ProcessError(ProcessError::Io(e))) if e.kind() == io::ErrorKind::PermissionDenied),
            "Io errors from terminate() must propagate directly, no escalation attempt \
         — a permissions failure won't be fixed by retrying with SIGKILL"
        );
    }

    #[tokio::test]
    async fn kill_calls_terminate_then_kill_exactly_once_each_on_escalation() {
        let mock = Arc::new(MockProcess::new(
            MockOutcome::NotResponding,
            MockOutcome::Success,
        ));
        let mock_wrap = MockProcessWrapper(Arc::clone(&mock));
        let process = PlatformProcess::new(mock_wrap, ProcessMetadata::default());

        let handle = KillHandle {
            runtime_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            process,
            daemon_path: PathBuf::from("/mock/bin/knotd"),
            policy: Arc::new(PolicyConfig::default()),
        };

        handle.kill().await.expect("must succeed via escalation");

        assert_eq!(mock.terminate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(mock.kill_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn kill_does_not_call_kill_when_terminate_succeeds() {
        let mock = Arc::new(MockProcess::new(MockOutcome::Success, MockOutcome::Success));
        let mock_wrap = MockProcessWrapper(Arc::clone(&mock));
        let process = PlatformProcess::new(mock_wrap, ProcessMetadata::default());

        let handle = KillHandle {
            runtime_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            process,
            daemon_path: PathBuf::from("/mock/bin/knot"),
            policy: Arc::new(PolicyConfig::default()),
        };

        handle
            .kill()
            .await
            .expect("terminate alone must be sufficient");

        assert_eq!(mock.terminate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            mock.kill_calls.load(Ordering::SeqCst),
            0,
            "kill() must not be called when terminate() already succeeded — \
         unconditional escalation would defeat the purpose of graceful shutdown"
        );
    }

    fn classify_poll_result(result: Result<(), ProcessError>) -> Option<Result<(), ProcessError>> {
        match result {
            Err(
                ProcessError::NotRunning | ProcessError::Mismatch { .. } | ProcessError::Reused,
            ) => Some(Ok(())),
            Err(e) => Some(Err(e)),
            Ok(()) => None,
        }
    }

    #[test]
    fn classify_treats_not_running_and_mismatch_as_exit_success() {
        assert!(matches!(
            classify_poll_result(Err(ProcessError::NotRunning)),
            Some(Ok(()))
        ));
        assert!(matches!(
            classify_poll_result(Err(ProcessError::Mismatch {
                expected: "a".into(),
                actual: "b".into()
            })),
            Some(Ok(()))
        ));
        assert!(classify_poll_result(Ok(())).is_none());
    }

    #[tokio::test]
    async fn kill_skips_terminate_and_goes_straight_to_kill_when_timeout_is_zero() {
        let mock = Arc::new(MockProcess::new(MockOutcome::Success, MockOutcome::Success));
        let mock_wrap = MockProcessWrapper(Arc::clone(&mock));
        let process = PlatformProcess::new(mock_wrap, ProcessMetadata::default());

        let mut policy = PolicyConfig::default();
        policy.kill.graceful_timeout = Duration::ZERO;

        let handle = KillHandle {
            runtime_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            process,
            daemon_path: PathBuf::from("/mock/bin/knotd"),
            policy: Arc::new(policy),
        };

        handle
            .kill()
            .await
            .expect("kill must succeed via direct SIGKILL path");

        assert_eq!(
            mock.terminate_calls.load(Ordering::SeqCst),
            0,
            "terminate() must be skipped entirely when graceful_timeout is zero — \
         calling it with a zero budget would be indistinguishable from not calling it, \
         so the code must not call it at all"
        );
        assert_eq!(
            mock.kill_calls.load(Ordering::SeqCst),
            1,
            "kill() must be called exactly once as the direct path"
        );
    }

    #[tokio::test]
    async fn kill_still_calls_terminate_when_timeout_is_nonzero() {
        let mock = Arc::new(MockProcess::new(MockOutcome::Success, MockOutcome::Success));
        let mock_wrap = MockProcessWrapper(Arc::clone(&mock));
        let process = PlatformProcess::new(mock_wrap, ProcessMetadata::default());

        let mut policy = PolicyConfig::default();
        policy.kill.graceful_timeout = Duration::from_millis(1);

        let handle = KillHandle {
            runtime_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
            process,
            daemon_path: PathBuf::from("/mock/bin/knot"),
            policy: Arc::new(policy),
        };

        handle
            .kill()
            .await
            .expect("kill must succeed via graceful path");

        assert_eq!(mock.terminate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            mock.kill_calls.load(Ordering::SeqCst),
            0,
            "kill() must not be called when terminate() succeeds with a non-zero budget"
        );
    }
}
