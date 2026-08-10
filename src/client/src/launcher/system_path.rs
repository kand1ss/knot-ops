use crate::errors::ClientError;
use crate::launcher::{DaemonLauncher, ExternalPathLauncher};
use async_trait::async_trait;
use std::path::Path;
use tracing::instrument;

/// A launcher that expects the `knot` binary to be available in the system PATH.
#[derive(Default)]
pub struct SystemPathLauncher {
    args: Vec<String>,
}

impl SystemPathLauncher {
    /// Creates a new `SystemPathLauncher`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a command-line argument to be passed to the daemon.
    pub fn arg(&mut self, value: impl Into<String>) -> &mut Self {
        self.args.push(value.into());
        self
    }
}

#[async_trait]
impl DaemonLauncher for SystemPathLauncher {
    #[instrument(skip_all, name = "system_path_launcher")]
    async fn launch(&self) -> Result<u32, ClientError> {
        let mut launcher = ExternalPathLauncher::new("knot");
        for arg in self.args.iter() {
            launcher.arg(arg);
        }
        launcher.launch().await
    }

    fn binary_path(&self) -> &Path {
        Path::new("knot")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_system_path_launcher_binary_path() {
        let launcher = SystemPathLauncher::new();
        assert_eq!(launcher.binary_path(), Path::new("knot"));
    }

    #[test]
    fn test_system_path_launcher_args() {
        let mut launcher = SystemPathLauncher::new();
        launcher.arg("--debug").arg("-v");
        assert_eq!(launcher.args, vec!["--debug", "-v"]);
    }
}
