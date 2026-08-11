use crate::errors::{ClientError, DaemonLifecycleError};
use crate::handles::ConnectedHandle;
use crate::policies::PolicyConfig;
use crate::process::Process;
use knot_core::consts::KNOT_SOCKET_FILE;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, instrument};

#[derive(Debug)]
pub struct OfflineHandle {
    pub runtime_dir: PathBuf,
    pub daemon_path: PathBuf,
    pub policy: Arc<PolicyConfig>,
}

impl OfflineHandle {
    /// Spawns the daemon process and waits for it to become ready.
    ///
    /// This method uses the configured [`DaemonLauncher`] to start the background process.
    /// After spawning, it aggressively tries to connect until the daemon
    /// creates the IPC socket and passes a full health check.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] (specifically a [`DaemonLifecycleError::LaunchFailed`])
    /// if the daemon process fails to start, or if the socket does not appear within
    /// the retry limit.    
    #[instrument(skip(self), name = "launch_daemon")]
    pub async fn launch(self) -> Result<ConnectedHandle, ClientError> {
        info!("spawning daemon process...");
        let _process = Process::spawn(&self.daemon_path).map_err(|e| {
            error!(error = %e, "failed to spawn daemon");
            ClientError::from(DaemonLifecycleError::LaunchFailed {
                message: "failed to execute daemon binary".to_string(),
                binary_path: self.daemon_path.to_string_lossy().into_owned(),
                error: e.to_string(),
            })
        })?;

        let mut retries = 40;
        let delay = Duration::from_millis(50);

        loop {
            if let Ok(handle) = ConnectedHandle::new(
                &self.runtime_dir.join(KNOT_SOCKET_FILE),
                Arc::clone(&self.policy),
            )
            .await
            {
                info!("daemon launched and healthy.");
                return Ok(handle);
            }

            retries -= 1;
            if retries == 0 {
                error!("daemon launch timeout: socket never appeared.");
                return Err(DaemonLifecycleError::LaunchFailed {
                    message: "daemon process was spawned, but IPC socket never appeared"
                        .to_string(),
                    binary_path: self.daemon_path.to_string_lossy().into_owned(),
                    error: "socket not found".to_string(),
                }
                .into());
            }

            debug!(
                "socket not ready yet, process is alive. Retrying in {}ms... (retries left: {})",
                delay.as_millis(),
                retries
            );
            tokio::time::sleep(delay).await;
        }
    }
}
