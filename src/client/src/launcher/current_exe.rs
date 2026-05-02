use crate::launcher::{DaemonLauncher, ExternalPathLauncher};
use async_trait::async_trait;
use knot_core::errors::{ClientError, DaemonLifecycleError};
use std::path::{Path, PathBuf};
use tracing::{error, instrument};

/// A launcher that uses the current executable to start the daemon.
///
/// This is useful when the same binary acts as both the client and the daemon.
#[derive(Default)]
pub struct CurrentExeLauncher {
    current_exe: PathBuf,
    args: Vec<String>,
}

impl CurrentExeLauncher {
    /// Creates a new `CurrentExeLauncher` using the current executable path.
    pub fn new() -> Self {
        let current_exe = std::env::current_exe().unwrap();
        Self {
            current_exe,
            args: Vec::new(),
        }
    }

    /// Adds a command-line argument to be passed to the daemon.
    pub fn arg(&mut self, value: impl Into<String>) -> &mut Self {
        self.args.push(value.into());
        self
    }
}

#[async_trait]
impl DaemonLauncher for CurrentExeLauncher {
    #[instrument(skip_all, name = "current_exe_launcher")]
    async fn launch(&self, directory: &Path) -> Result<u32, ClientError> {
        let current_exe = std::env::current_exe().map_err(|e| {
            error!(error = %e, "Failed to get current executable path");
            DaemonLifecycleError::LaunchFailed {
                message: "Failed to get current executable path".to_string(),
                binary_path: "?".to_string(),
                target_dir: directory.to_string_lossy().into_owned(),
                error: e.to_string(),
            }
        })?;
        let mut launcher = ExternalPathLauncher::new(current_exe);
        for arg in self.args.iter() {
            launcher.arg(arg);
        }
        launcher.launch(directory).await
    }

    fn binary_path(&self) -> &Path {
        &self.current_exe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_exe_launcher_binary_path() {
        let launcher = CurrentExeLauncher::new();
        assert_eq!(
            launcher.binary_path(),
            std::env::current_exe().unwrap().as_path()
        );
    }

    #[test]
    fn test_current_exe_launcher_args() {
        let mut launcher = CurrentExeLauncher::new();
        launcher.arg("--mode").arg("test");
        assert_eq!(launcher.args, vec!["--mode", "test"]);
    }
}
