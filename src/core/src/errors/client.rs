use crate::errors::TransportError;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),

    #[error(transparent)]
    Daemon(#[from] DaemonLifecycleError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error(transparent)]
    Healthcheck(#[from] HealthcheckError),

    #[error(transparent)]
    Transport(#[from] TransportError),
}

impl ClientError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::Workspace(workspace) => workspace.context(),
            Self::Daemon(daemon) => daemon.context(),
            Self::Protocol(protocol) => protocol.context(),
            Self::Healthcheck(health) => health.context(),
            Self::Transport(transport) => Some(transport.to_string()),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::Workspace(workspace) => workspace.solution(),
            Self::Daemon(daemon) => daemon.solution(),
            Self::Protocol(protocol) => protocol.solution(),
            Self::Healthcheck(health) => health.solution(),
            Self::Transport(_) => None,
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
        target_dir: String,
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
                target_dir,
                message,
                error,
            } => Some(format!(
                "attempted to launch '{}' in '{}'.\ndetails: {}\nsystem error: {}",
                binary_path,
                target_dir.replace("\\\\?\\", ""),
                message,
                error
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

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("the daemon could not execute the requested command")]
    CommandFailed(String),

    #[error("received an unexpected response from the daemon")]
    UnexpectedResponse { expected: String },
}

impl ProtocolError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::CommandFailed(details) => Some(format!("command execution details: {}", details)),
            Self::UnexpectedResponse { expected } => Some(format!(
                "expected to receive a '{}' message type, but got something else",
                expected
            )),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::CommandFailed(_) => {
                Some("check the daemon logs for more details using 'knot logs'.")
            }
            Self::UnexpectedResponse { .. } => {
                Some("ensure your knot cli and daemon versions match.")
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum HealthcheckError {
    #[error("cannot reach the knot daemon")]
    NotConnected,

    #[error("found a dead connection to the daemon")]
    StaleSocket(PathBuf),

    #[error("the daemon's internal state is inconsistent")]
    InconsistentState(String),

    #[error("the daemon process has unexpectedly disappeared")]
    ProcessNotExists(u32),

    #[error("the daemon process is unresponsive")]
    ZombieProcess(u32),

    #[error("the daemon is connected but ignoring requests")]
    DaemonNotResponding,
}

impl HealthcheckError {
    pub fn context(&self) -> Option<String> {
        match self {
            Self::NotConnected => {
                Some("the client transport is missing or disconnected.".to_string())
            }
            Self::StaleSocket(path) => Some(format!(
                "a stale socket file was left behind at '{}' from a previous crash",
                path.display()
            )),
            Self::InconsistentState(details) => Some(format!("state conflict: {}", details)),
            Self::ProcessNotExists(pid) => Some(format!(
                "expected to find an active process with PID {}, but it's gone",
                pid
            )),
            Self::ZombieProcess(pid) => {
                Some(format!("process {} is in a zombie or hung state", pid))
            }
            Self::DaemonNotResponding => Some("the ping healthcheck timed out.".to_string()),
        }
    }

    pub fn solution(&self) -> Option<&'static str> {
        match self {
            Self::NotConnected => Some("run 'knot up' to start the environment."),
            Self::StaleSocket(_) => Some("run 'knot repair' to clean up the environment."),
            Self::InconsistentState(_) => Some("run 'knot repair' to reset the workspace state."),
            Self::ProcessNotExists(_) => Some("run 'knot repair' to clean up the stale PID file."),
            Self::ZombieProcess(_) => Some("kill the process manually or run 'knot repair'."),
            Self::DaemonNotResponding => Some("restart the daemon with 'knot down' and 'knot up'."),
        }
    }
}
