use knot_core::errors::PathResolutionError;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),

    #[error(transparent)]
    Daemon(#[from] DaemonLifecycleError),

    #[error("daemon request failed")]
    Protocol(#[from] tonic::Status),

    #[error("failed to establish transport channel")]
    Transport(#[from] tonic::transport::Error),

    #[error("filesystem error")]
    Io(#[from] std::io::Error),

    #[error("protocol contract violation")]
    Contract(String),

    #[error("failed to resolve daemon binary path")]
    PathResolution(#[from] PathResolutionError),
}

impl ClientError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::Workspace(workspace) => workspace.context(),
            Self::Daemon(daemon) => daemon.context(),
            Self::Protocol(status) => Some(status.message().to_string()),
            Self::Transport(e) => Some(e.to_string()),
            Self::Io(e) => Some(e.to_string()),
            Self::Contract(msg) => Some(msg.clone()),
            Self::PathResolution(_) => Some(
                "the OS reported no valid application data directory for the current user"
                    .to_string(),
            ),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::Workspace(workspace) => workspace.solution(),
            Self::Daemon(daemon) => daemon.solution(),
            Self::Contract(_) => Some("ensure your CLI and daemon versions match."),
            Self::Transport(_) => {
                Some("the daemon might have crashed. Try running 'knot repair' or check logs.")
            }
            Self::Protocol(status) => match status.code() {
                tonic::Code::Unavailable => Some(
                    "the daemon is unreachable. It may have crashed or was shut down. Run 'knot repair'.",
                ),
                tonic::Code::Unimplemented => Some(
                    "version mismatch: the daemon does not support this command. Update your CLI or Daemon.",
                ),
                tonic::Code::FailedPrecondition => {
                    Some("the workspace is not in the correct state to execute this command.")
                }
                tonic::Code::PermissionDenied => Some(
                    "you do not have the required permissions to execute this request via the IPC socket.",
                ),
                _ => Some("check the daemon logs for more details using 'knot logs'."),
            },
            Self::Io(e) => match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    Some("verify file permissions or run the command with elevated privileges.")
                }
                std::io::ErrorKind::NotFound => {
                    Some("a required file or directory was not found. check your workspace paths.")
                }
                std::io::ErrorKind::AlreadyExists => Some(
                    "a file or socket already exists where knot is trying to create one. try running 'knot repair'.",
                ),
                std::io::ErrorKind::AddrInUse => {
                    Some("the required port or IPC socket is already in use by another process.")
                }
                _ => Some(
                    "verify disk space and directory permissions, or try running 'knot repair'.",
                ),
            },
            Self::PathResolution(_) => Some(
                "Ensure your environment has a valid \
                home directory configured (e.g. the $HOME variable on Linux/macOS, \
                or a valid user profile on Windows), then retry.",
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("knot workspace is not initialized")]
    NotInitialized(String),

    #[error("workspace data is corrupted or unreadable")]
    BrokenData(PathBuf),
}

impl WorkspaceError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::NotInitialized(path) => Some(format!(
                "could not find the '.knot' directory at '{}'",
                path
            )),
            Self::BrokenData(path) => Some(format!(
                "the file '{}' contains unexpected or broken data",
                path.display()
            )),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::NotInitialized(_) => {
                Some("run 'knot init' to set up a new workspace in this directory.")
            }
            Self::BrokenData(_) => {
                Some("try running 'knot repair' or delete the corrupted file to recreate it.")
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum DaemonLifecycleError {
    #[error("the knot background daemon is not running")]
    NotRunning(PathBuf),

    #[error("failed to start the knot background daemon")]
    LaunchFailed {
        message: String,
        binary_path: String,
        error: String,
    },
}

impl DaemonLifecycleError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::NotRunning(path) => Some(format!(
                "expected an active socket file at '{}', but it wasn't found",
                path.display()
            )),
            Self::LaunchFailed {
                binary_path,
                message,
                error,
            } => Some(format!(
                "attempted to launch '{}'.\ndetails: {}\nsystem error: {}",
                binary_path, message, error
            )),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::NotRunning(_) => Some("start the daemon by running 'knot up'."),
            Self::LaunchFailed { .. } => {
                Some("ensure the knot executable is present in your PATH or explicitly configured.")
            }
        }
    }
}
