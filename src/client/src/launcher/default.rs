use crate::errors::{ClientError, DaemonLifecycleError};
use crate::launcher::{DaemonLauncher, StandardDaemonLauncher};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// The default launcher used by `KnotClient`.
///
/// It launches `knotd` from Knot's standardized per-user binary directory.
pub(crate) struct DefaultLauncher {
    default_path: PathBuf,
}

impl DefaultLauncher {
    /// Creates a new `DefaultLauncher`.
    pub fn new() -> Self {
        let default_path = knot_core::paths::daemon_binary_path()
            .unwrap_or_else(|| PathBuf::from(knot_core::paths::KNOT_DAEMON_BINARY_NAME));

        Self { default_path }
    }
}

#[async_trait]
impl DaemonLauncher for DefaultLauncher {
    #[instrument(skip_all, name = "default_launcher")]
    async fn launch(&self) -> Result<u32, ClientError> {
        let launcher = StandardDaemonLauncher::new()?;

        launcher.launch().await.map_err(|e| DaemonLifecycleError::LaunchFailed {
            message: "The knot daemon was not found. Install knotd into the standard Knot binary directory or manually specify the daemon executable path.".to_string(),
            binary_path: launcher.binary_path().to_string_lossy().into_owned(),
            error: e.to_string()
        }.into())
    }

    fn binary_path(&self) -> &Path {
        &self.default_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_launcher_binary_path() {
        let launcher = DefaultLauncher::new();
        let path = launcher.binary_path();
        assert_eq!(
            path.file_name().unwrap(),
            knot_core::paths::KNOT_DAEMON_BINARY_NAME
        );
    }
}
