use crate::process::{Process, ProcessError};
use crate::{errors::ClientError, handles::*, policies::*, states::ConnectState};
use knot_core::consts::{KNOT_DAEMON_LOCK_FILE, KNOT_SOCKET_FILE};
use knot_core::paths::{KNOT_DAEMON_BINARY_NAME, daemon_binary_path, daemon_runtime_dir};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, instrument, trace, warn};

/// A builder interface for configuring and bootstrapping a connection to the Knot daemon.
///
/// `ClientBuilder` allows you to customize the initialization parameters, such as the
/// strategy used to spawn the background daemon process, before executing the state
/// resolution sequence via [`Self::connect`].
pub struct ClientBuilder {
    runtime_dir: PathBuf,
    daemon_path: PathBuf,
    policy: PolicyConfig,
}
impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            runtime_dir: daemon_runtime_dir(),
            daemon_path: daemon_binary_path().unwrap(), // TODO - handle this better
            policy: PolicyConfig::default(),
        }
    }
}
impl ClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, timeout: TimeoutPolicy) -> Self {
        self.policy.timeout = timeout;
        self
    }

    pub fn with_daemon_path(mut self, path: impl AsRef<Path>) -> Self {
        self.daemon_path = path.as_ref().to_owned();
        self
    }

    pub fn with_runtime_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.runtime_dir = path.as_ref().to_owned();
        self
    }

    /// Resolves the workspace environment and evaluates the daemon's current operational state.
    ///
    /// This method performs a multistep inspection to determine how to connect to the daemon:
    /// 1. Recursively searches upwards to find the `.knot` workspace directory.
    /// 2. Inspects the filesystem for existing PID and socket files.
    /// 3. Cross-references the discovered PID with the operating system's process table.
    /// 4. Attempts to establish a gRPC connection if the process and socket are deemed healthy.
    ///
    /// This method consumes the builder to transfer ownership of the `launcher` configuration
    /// to the resulting state handle.
    ///
    /// # Arguments
    ///
    /// * `directory` - The starting path to begin searching for the workspace root.
    ///
    /// # Returns
    ///
    /// Returns a `ConnectState` enum representing the exact lifecycle phase of the daemon
    /// (e.g., `Offline`, `Connected`, `Hung`, or `Stale`).
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` (specifically `WorkspaceError::NotInitialized`) if no valid
    /// workspace directory is found in the path hierarchy.
    #[instrument(skip_all)]
    pub async fn connect(self) -> Result<ConnectState, ClientError> {
        debug!("starting connection sequence...");

        let policy = Arc::new(self.policy);
        let runtime_dir = self.runtime_dir;
        let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

        let handle = match (lock_path.exists(), socket_path.exists()) {
            (false, false) => {
                debug!("daemon artifacts not found. Daemon is offline.");

                ConnectState::Offline(OfflineHandle {
                    runtime_dir,
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                })
            }
            (true, true) => {
                debug!("daemon artifacts found. Daemon is online.");
                let handle = ConnectedHandle::new(&socket_path, Arc::clone(&policy)).await;
                if let Ok(handle) = handle {
                    trace!("successfully connected to running daemon.");
                    ConnectState::Connected(handle)
                } else {
                    warn!(
                        "daemon artifacts found but connection is failed. Checking for hung or stale state."
                    );
                    if let Some(daemon_pid) = Self::read_as_u32(&lock_path).await {
                        let expected_daemon_binary = KNOT_DAEMON_BINARY_NAME.to_string();
                        match Process::bind(daemon_pid, expected_daemon_binary.clone()).await {
                            Ok(process) => {
                                debug!(
                                    pid = daemon_pid,
                                    "process at this PID is a valid knot daemon. Proceeding to kill it."
                                );
                                ConnectState::Hung(KillHandle {
                                    runtime_dir,
                                    process: Box::new(process),
                                    daemon_path: self.daemon_path,
                                    policy: Arc::clone(&policy),
                                })
                            }
                            Err(ProcessError::NotRunning) => {
                                warn!(
                                    pid = daemon_pid,
                                    expected = expected_daemon_binary,
                                    "process does not exist; assuming daemon is already dead. Treating as Stale."
                                );
                                ConnectState::Stale(StaleHandle {
                                    runtime_dir,
                                    daemon_path: self.daemon_path,
                                    policy: Arc::clone(&policy),
                                })
                            }
                            Err(ProcessError::Mismatch(actual)) => {
                                warn!(
                                    pid = daemon_pid,
                                    actual = %actual,
                                    expected = expected_daemon_binary,
                                    "process at this PID does not match the expected knot binary; refusing to kill it. Treating as Stale."
                                );
                                ConnectState::Stale(StaleHandle {
                                    runtime_dir,
                                    daemon_path: self.daemon_path,
                                    policy: Arc::clone(&policy),
                                })
                            }
                        }
                    } else {
                        warn!("PID file exists but is corrupted or empty. Treating as Stale.");
                        ConnectState::Stale(StaleHandle {
                            runtime_dir,
                            daemon_path: self.daemon_path,
                            policy: Arc::clone(&policy),
                        })
                    }
                }
            }
            _ => {
                warn!("Not all daemon artifacts found. Treating as Stale.");

                ConnectState::Stale(StaleHandle {
                    runtime_dir,
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                })
            }
        };
        Ok(handle)
    }

    async fn read_as_u32(path: &Path) -> Option<u32> {
        let content = tokio::fs::read_to_string(path).await.ok()?;
        content.trim().parse::<u32>().ok()
    }
}
