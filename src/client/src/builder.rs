use crate::process::{Process, ProcessVerification};
use crate::{
    errors::ClientError,
    handles::*,
    launcher::{DaemonLauncher, DefaultLauncher},
    policies::*,
    states::ConnectState,
};
use knot_core::consts::{KNOT_DAEMON_LOCK_FILE, KNOT_SOCKET_FILE};
use knot_core::paths::{KNOT_DAEMON_BINARY_NAME, daemon_runtime_dir};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, instrument, trace, warn};

/// A builder interface for configuring and bootstrapping a connection to the Knot daemon.
///
/// `ClientBuilder` allows you to customize the initialization parameters, such as the
/// strategy used to spawn the background daemon process, before executing the state
/// resolution sequence via [`Self::connect`].
pub struct ClientBuilder {
    launcher: Box<dyn DaemonLauncher + Send + Sync + 'static>,
    policy: PolicyConfig,
}
impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            launcher: Box::new(DefaultLauncher::new()),
            policy: PolicyConfig::default(),
        }
    }
}
impl ClientBuilder {
    /// Overrides the default daemon launch strategy.
    ///
    /// This is particularly useful for injecting mock launchers during testing or
    /// explicitly defining whether to use the system path versus the current executable.
    ///
    /// # Arguments
    ///
    /// * `launcher` - A type that implements the `DaemonLauncher` trait.
    pub fn with_launcher(mut self, launcher: impl DaemonLauncher + Send + Sync + 'static) -> Self {
        self.launcher = Box::new(launcher);
        self
    }

    pub fn with_timeout(mut self, timeout: TimeoutPolicy) -> Self {
        self.policy.timeout = timeout;
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
        let runtime_dir = daemon_runtime_dir();
        let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

        let handle = match (lock_path.exists(), socket_path.exists()) {
            (false, false) => {
                debug!("daemon artifacts not found. Daemon is offline.");

                ConnectState::Offline(OfflineHandle {
                    runtime_dir: runtime_dir,
                    daemon_launcher: self.launcher,
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
                        let process = Process::new(daemon_pid, KNOT_DAEMON_BINARY_NAME.to_string());
                        match process.verify() {
                            ProcessVerification::Valid => {
                                debug!(
                                    pid = daemon_pid,
                                    "process at this PID is a valid knot daemon. Proceeding to kill it."
                                );
                                ConnectState::Hung(KillHandle {
                                    runtime_dir: runtime_dir,
                                    process: Box::new(process),
                                    daemon_launcher: self.launcher,
                                    policy: Arc::clone(&policy),
                                })
                            }
                            ProcessVerification::NotRunning => {
                                warn!(
                                    pid = process.pid,
                                    expected = process.name,
                                    "process does not exist; assuming daemon is already dead. Treating as Stale."
                                );
                                ConnectState::Stale(StaleHandle {
                                    runtime_dir: runtime_dir,
                                    daemon_launcher: self.launcher,
                                    policy: Arc::clone(&policy),
                                })
                            }
                            ProcessVerification::Mismatch(actual) => {
                                warn!(
                                    pid = process.pid,
                                    actual = %actual,
                                    expected = process.name,
                                    "process at this PID does not match the expected knot binary; refusing to kill it. Treating as Stale."
                                );
                                ConnectState::Stale(StaleHandle {
                                    runtime_dir: runtime_dir,
                                    daemon_launcher: self.launcher,
                                    policy: Arc::clone(&policy),
                                })
                            }
                        }
                    } else {
                        warn!("PID file exists but is corrupted or empty. Treating as Stale.");
                        ConnectState::Stale(StaleHandle {
                            runtime_dir: runtime_dir,
                            daemon_launcher: self.launcher,
                            policy: Arc::clone(&policy),
                        })
                    }
                }
            }
            _ => {
                warn!("Not all daemon artifacts found. Treating as Stale.");

                ConnectState::Stale(StaleHandle {
                    runtime_dir: runtime_dir,
                    daemon_launcher: self.launcher,
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
