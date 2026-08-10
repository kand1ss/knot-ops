use crate::errors::{ClientError, DaemonLifecycleError};
use crate::launcher::{DaemonLauncher, ExternalPathLauncher};
use async_trait::async_trait;
use knot_core::paths;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Launches `knotd` from Knot's standardized per-user binary directory.
pub struct StandardDaemonLauncher {
    binary_path: PathBuf,
    args: Vec<String>,
}

impl StandardDaemonLauncher {
    pub fn new() -> Result<Self, ClientError> {
        let binary_path =
            paths::daemon_binary_path().ok_or_else(|| DaemonLifecycleError::LaunchFailed {
                message: "Failed to resolve the standard Knot binary directory".to_string(),
                binary_path: paths::KNOT_DAEMON_BINARY_NAME.to_string(),
                error: "directories::ProjectDirs returned None".to_string(),
            })?;

        Ok(Self {
            binary_path,
            args: Vec::new(),
        })
    }

    pub fn arg(&mut self, value: impl Into<String>) -> &mut Self {
        self.args.push(value.into());
        self
    }
}

#[async_trait]
impl DaemonLauncher for StandardDaemonLauncher {
    #[instrument(skip_all, name = "standard_daemon_launcher")]
    async fn launch(&self) -> Result<u32, ClientError> {
        if !self.binary_path.is_file() {
            return Err(DaemonLifecycleError::LaunchFailed {
                message:
                    "The knot daemon binary was not found in the standard Knot binary directory"
                        .to_string(),
                binary_path: self.binary_path.to_string_lossy().into_owned(),
                error: "file does not exist".to_string(),
            }
            .into());
        }

        let mut launcher = ExternalPathLauncher::new(&self.binary_path);
        for arg in self.args.iter() {
            launcher.arg(arg);
        }

        launcher.launch().await
    }

    fn binary_path(&self) -> &Path {
        &self.binary_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_daemon_launcher_binary_path() {
        let launcher = StandardDaemonLauncher::new().unwrap();
        assert_eq!(
            launcher.binary_path().file_name().unwrap(),
            paths::KNOT_DAEMON_BINARY_NAME
        );
    }

    #[test]
    fn test_standard_daemon_launcher_args() {
        let mut launcher = StandardDaemonLauncher::new().unwrap();
        launcher.arg("--debug").arg("-v");
        assert_eq!(launcher.args, vec!["--debug", "-v"]);
    }
}
