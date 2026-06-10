use crate::launcher::{DaemonLauncher, ExternalPathLauncher};
use async_trait::async_trait;
use directories::ProjectDirs;
use knot_core::errors::{ClientError, DaemonLifecycleError};
use std::path::{Path, PathBuf};
use tracing::instrument;

/// The default launcher used by `KnotClient`.
///
/// It attempts to find the `knot` binary in the project's data directory
/// or falls back to the system PATH.
pub(crate) struct DefaultLauncher {
    default_path: PathBuf,
}

impl DefaultLauncher {
    /// Creates a new `DefaultLauncher`.
    pub fn new() -> Self {
        let default_path = if let Some(proj_dirs) = ProjectDirs::from("", "", "knot") {
            let mut path = proj_dirs.data_dir().to_path_buf();
            path.push("bin");
            path.push(if cfg!(windows) { "knot.exe" } else { "knot" });
            path
        } else {
            PathBuf::from("knot")
        };

        Self { default_path }
    }
}

#[async_trait]
impl DaemonLauncher for DefaultLauncher {
    #[instrument(skip_all, name = "default_launcher")]
    async fn launch(&self, directory: &Path) -> Result<u32, ClientError> {
        let launcher = if self.default_path.exists() {
            ExternalPathLauncher::new(&self.default_path)
        } else {
            ExternalPathLauncher::new("knot")
        };

        launcher.launch(directory).await.map_err(|e| DaemonLifecycleError::LaunchFailed {
            message: "The knot utility was not found. Make sure you have knot installed, or manually specify the path to the knot executable.".to_string(), 
            binary_path: launcher.binary_path().to_string_lossy().into_owned(),
            target_dir: directory.to_string_lossy().into_owned(),
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
        assert!(path.components().count() > 0);
    }
}
