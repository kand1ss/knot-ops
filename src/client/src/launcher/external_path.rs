use crate::errors::ClientError;
use crate::launcher::{DaemonLauncher, spawn_process};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// A launcher that uses a specific daemon executable path.
pub struct ExternalPathLauncher {
    binary_file_path: PathBuf,
    args: Vec<String>,
}

impl ExternalPathLauncher {
    /// Creates a new `ExternalPathLauncher` with the specified binary path.
    pub fn new(binary_file_path: impl AsRef<Path>) -> Self {
        Self {
            binary_file_path: binary_file_path.as_ref().to_path_buf(),
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
impl DaemonLauncher for ExternalPathLauncher {
    #[instrument(skip_all, name = "external_path_launcher")]
    async fn launch(&self) -> Result<u32, ClientError> {
        spawn_process(&self.binary_file_path, &self.args)
    }

    fn binary_path(&self) -> &Path {
        &self.binary_file_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_external_path_launcher_binary_path() {
        let path = PathBuf::from("/usr/bin/custom_knot");
        let launcher = ExternalPathLauncher::new(&path);
        assert_eq!(launcher.binary_path(), path);
    }

    #[test]
    fn test_external_path_launcher_args() {
        let mut launcher = ExternalPathLauncher::new("custom_knot");
        launcher.arg("--config").arg("custom.toml");
        assert_eq!(launcher.args, vec!["--config", "custom.toml"]);
    }
}
