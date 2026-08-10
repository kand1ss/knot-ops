use crate::errors::{ClientError, DaemonLifecycleError};
use async_trait::async_trait;
use std::path::Path;
use tokio::process::Command;
use tracing::{debug, error, info, instrument};

mod external_path;
pub use external_path::*;
mod standard_daemon;
pub use standard_daemon::*;
mod system_path;
pub use system_path::*;
mod default;
pub(crate) use default::*;

/// A trait for launching the `knot` daemon process.
///
/// Different implementations can be used to locate and spawn the daemon executable
/// from various sources (e.g., current executable, system PATH, or a specific path).
#[async_trait]
pub trait DaemonLauncher {
    /// Launches the daemon process in the specified directory.
    ///
    /// Returns the PID of the spawned process on success.
    async fn launch(&self) -> Result<u32, ClientError>;

    /// Returns the path to the daemon binary.
    fn binary_path(&self) -> &Path;
}

#[instrument(
    skip_all,
    name = "daemon_process_spawn",
    fields(
        bin = %binary_file.display(),
        args = ?args
    )
)]
pub(crate) fn spawn_process(
    binary_file: &Path,
    args: &[String],
) -> Result<u32, ClientError> {
    let mut command = Command::new(binary_file);

    for arg in args.iter() {
        command.arg(arg);
    }

    debug!("Spawning daemon...");

    let child = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e: std::io::Error| {
            error!(error = %e, "Failed to spawn daemon executable");
            ClientError::Daemon(DaemonLifecycleError::LaunchFailed {
                message: "Failed to spawn daemon executable".to_string(),
                binary_path: binary_file.to_string_lossy().into_owned(),
                error: e.to_string(),
            })
        })?;

    match child.id() {
        Some(id) => {
            info!("Daemon successfully spawned with PID: {}", id);
            Ok(id)
        }
        None => {
            error!("Daemon process exited immediately after spawning.");
            Err(DaemonLifecycleError::LaunchFailed {
                message: "Daemon process exited immediately and yielded no PID. It might have crashed on startup.".to_string(),
                binary_path: binary_file.to_string_lossy().into_owned(),
                error: String::from(""),
            }.into())
        }
    }
}