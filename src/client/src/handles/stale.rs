use crate::{handles::OfflineHandle, launcher::DaemonLauncher, policies::PolicyConfig};
#[cfg(not(windows))]
use knot_core::consts::KNOT_SOCKET_FILE;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info, instrument};
use knot_core::consts::KNOT_DAEMON_LOCK_FILE;

/// A transitional handle representing a workspace with stale daemon artifacts.
///
/// This state occurs when the operating system confirms that the daemon process
/// is no longer running, but volatile files (such as the PID file or UNIX domain socket)
/// were left behind on the filesystem (e.g., due to a panic, power loss, or a forced `SIGKILL`).
/// The only permitted operation in this state is purging these artifacts via [`Self::clean`].
pub struct StaleHandle {
    pub(crate) runtime_dir: PathBuf,
    pub(crate) daemon_launcher: Box<dyn DaemonLauncher + Send + Sync>,
    pub(crate) policy: Arc<PolicyConfig>,
}

impl StaleHandle {
    #[cfg(not(windows))]
    fn socket_path(&self) -> PathBuf {
        self.runtime_dir.join(KNOT_SOCKET_FILE)
    }

    fn daemon_lock_path(&self) -> PathBuf {
        self.runtime_dir.join(KNOT_DAEMON_LOCK_FILE)
    }

    /// Purges stale daemon artifacts to prepare the daemon for a fresh launch.
    ///
    /// This method consumes the `StaleHandle` and attempts to safely delete the orphaned
    /// socket and PID files. Upon successful cleanup, it transitions the session into
    /// an `OfflineHandle`, which can then be used to safely bind a new socket and bootstrap a new daemon.
    ///
    /// # Returns
    ///
    /// Returns an `OfflineHandle` representing a clean filesystem ready for daemon initialization.
    ///
    /// # Errors
    ///
    /// Returns an `std::io::Error` if the filesystem lacks the necessary permissions to
    /// delete the files, or if an underlying OS error occurs during deletion.
    #[instrument(skip(self), fields(dir = %self.runtime_dir.display()))]
    pub async fn clean(self) -> io::Result<OfflineHandle> {
        debug!("initiating cleanup of stale daemon artifacts");

        #[cfg(not(windows))]
        {
            let sock_path = self.socket_path();
            if sock_path.exists() {
                debug!(path = %sock_path.display(), "removing orphaned socket file");
                tokio::fs::remove_file(&sock_path).await.map_err(|e| {
                    error!(error = %e, path = %sock_path.display(), "failed to remove socket file");
                    e
                })?;
            } else {
                debug!("socket file not found, skipping removal");
            }
        }

        let lock_path = self.daemon_lock_path();
        if lock_path.exists() {
            debug!(path = %lock_path.display(), "removing stale lock file");
            tokio::fs::remove_file(&lock_path).await.map_err(|e| {
                error!(error = %e, path = %lock_path.display(), "failed to remove lock file");
                e
            })?;
        } else {
            debug!("lock file not found, skipping removal");
        }

        info!("volatile files successfully cleaned up, transitioning to offline state");

        Ok(OfflineHandle {
            runtime_dir: self.runtime_dir,
            daemon_launcher: self.daemon_launcher,
            policy: Arc::clone(&self.policy),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ClientError;
    use async_trait::async_trait;
    use std::path::Path;

    struct DummyLauncher;

    #[async_trait]
    impl DaemonLauncher for DummyLauncher {
        async fn launch(&self) -> Result<u32, ClientError> {
            Ok(1)
        }

        fn binary_path(&self) -> &Path {
            Path::new("/bin/knotd")
        }
    }

    fn handle(dir: PathBuf) -> StaleHandle {
        StaleHandle {
            runtime_dir: dir,
            daemon_launcher: Box::new(DummyLauncher),
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    #[tokio::test]
    async fn test_stale_clean() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path().to_path_buf();

        let lock_path = dir.join(KNOT_DAEMON_LOCK_FILE);
        tokio::fs::write(&lock_path, "1234").await.unwrap();

        #[cfg(not(windows))]
        let socket_path = dir.join(KNOT_SOCKET_FILE);
        #[cfg(not(windows))]
        tokio::fs::write(&socket_path, "").await.unwrap();

        let handle = handle(dir.clone());
        let offline_handle = handle.clean().await.unwrap();

        assert_eq!(offline_handle.runtime_dir, dir);
        assert!(!lock_path.exists());

        #[cfg(not(windows))]
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn test_stale_clean_no_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path().to_path_buf();

        let lock_path = dir.join(knot_core::consts::KNOT_DAEMON_LOCK_FILE);
        #[cfg(not(windows))]
        let socket_path = dir.join(knot_core::consts::KNOT_SOCKET_FILE);

        let handle = handle(dir.clone());
        let offline_handle = handle.clean().await.unwrap();

        assert_eq!(offline_handle.runtime_dir, dir);
        assert!(!lock_path.exists());

        #[cfg(not(windows))]
        assert!(!socket_path.exists());
    }
}
