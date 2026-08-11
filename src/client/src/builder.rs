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
    expected_daemon_name: String,
    policy: PolicyConfig,
}
impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            runtime_dir: daemon_runtime_dir(),
            daemon_path: daemon_binary_path().unwrap(), // TODO - handle this better
            expected_daemon_name: KNOT_DAEMON_BINARY_NAME.to_string(),
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

    pub fn with_expected_daemon_name(mut self, name: impl Into<String>) -> Self {
        self.expected_daemon_name = name.into();
        self
    }

    /// This method performs a multistep inspection to determine how to connect to the daemon:
    /// 1. Checks whether the daemon lock file exists.
    /// 2. Attempts to establish an IPC connection to the daemon.
    /// 3. If the connection fails, reads the daemon PID from the lock file.
    /// 4. Cross-references the PID with the operating system's process table.
    /// 5. Classifies the daemon as hung or stale based on the process state.
    #[instrument(skip_all)]
    pub async fn connect(self) -> Result<ConnectState, ClientError> {
        debug!("starting connection sequence...");

        let policy = Arc::new(self.policy);
        let runtime_dir = self.runtime_dir;
        let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

        // The lock file is the daemon's lifecycle marker.
        //
        // We deliberately do NOT check socket_path.exists():
        // on Windows the IPC endpoint may be a named pipe rather than
        // a filesystem entry, so filesystem existence is not a reliable
        // indication that the endpoint is available.
        if !lock_path.exists() {
            #[cfg(unix)]
            {
                if socket_path.exists() {
                    warn!(
                        "daemon lock file not found but socket file exists. \
                         Treating as stale."
                    );
                    return Ok(ConnectState::Stale(StaleHandle {
                        runtime_dir,
                        daemon_path: self.daemon_path,
                        policy: Arc::clone(&policy),
                    }));
                }
            }

            debug!("daemon lock file not found. Daemon is offline.");
            return Ok(ConnectState::Offline(OfflineHandle {
                runtime_dir,
                daemon_path: self.daemon_path,
                policy: Arc::clone(&policy),
            }));
        }

        debug!("daemon lock file found. Attempting to connect to daemon.");

        // IPC availability is determined by an actual connection attempt,
        // not by checking the endpoint's filesystem existence.
        match ConnectedHandle::new(&socket_path, Arc::clone(&policy)).await {
            Ok(handle) => {
                trace!("successfully connected to running daemon.");

                return Ok(ConnectState::Connected(handle));
            }

            Err(_) => {
                warn!(
                    "daemon lock file exists but connection failed. \
                 Checking for hung or stale state."
                );
            }
        }

        let daemon_pid = match Self::read_as_u32(&lock_path).await {
            Some(pid) => pid,
            None => {
                warn!("PID file exists but is corrupted or empty. Treating as stale.");

                return Ok(ConnectState::Stale(StaleHandle {
                    runtime_dir,
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                }));
            }
        };

        debug!(
            pid = daemon_pid,
            expected = %self.expected_daemon_name,
            "binding to daemon process"
        );

        let expected_name = self.expected_daemon_name.clone();

        match Process::bind(daemon_pid, expected_name.clone()).await {
            Ok(process) => {
                debug!(
                    pid = daemon_pid,
                    expected = %expected_name,
                    "process at this PID is a valid knot daemon"
                );

                Ok(ConnectState::Hung(KillHandle {
                    runtime_dir,
                    process: Box::new(process),
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                }))
            }

            Err(ProcessError::NotRunning) => {
                warn!(
                    pid = daemon_pid,
                    expected = %expected_name,
                    "process does not exist; treating as stale"
                );

                Ok(ConnectState::Stale(StaleHandle {
                    runtime_dir,
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                }))
            }

            Err(ProcessError::Mismatch { expected, actual }) => {
                warn!(
                    pid = daemon_pid,
                    actual = %actual,
                    expected = %expected,
                    "process at this PID does not match the expected daemon binary; \
                     treating as stale"
                );

                Ok(ConnectState::Stale(StaleHandle {
                    runtime_dir,
                    daemon_path: self.daemon_path,
                    policy: Arc::clone(&policy),
                }))
            }
        }
    }

    async fn read_as_u32(path: &Path) -> Option<u32> {
        let content = tokio::fs::read_to_string(path).await.ok()?;
        content.trim().parse::<u32>().ok()
    }
}
