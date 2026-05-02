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

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("directory .knot was not found at '{0}'")]
    NotInitialized(String),

    #[error("the file '{0}' exists but contains unexpected or corrupted data")]
    BrokenData(PathBuf),
}

#[derive(Debug, Error)]
pub enum DaemonLifecycleError {
    #[error("daemon is not running (socket not found at {0})")]
    NotRunning(PathBuf),

    #[error(
        "daemon launch failed at '{target_dir}'\nBinary: {binary_path}\nError: {error}\nDetails: {message}"
    )]
    LaunchFailed {
        message: String,
        binary_path: String,
        target_dir: String,
        error: String,
    },
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("failed to execute command on daemon: {0}")]
    CommandFailed(String),

    #[error("unexpected response type from daemon (expected: {expected})")]
    UnexpectedResponse { expected: String },
}

#[derive(Debug, Error)]
pub enum HealthcheckError {
    #[error("Client is not connected to any transport")]
    NotConnected,

    #[error("Socket exists at {0}, but it's dead (stale)")]
    StaleSocket(PathBuf),

    #[error("Inconsistent state: {0}")]
    InconsistentState(String),

    #[error("Process with PID {0} does not exist")]
    ProcessNotExists(u32),

    #[error("Process with PID {0} is a zombie or unresponsive")]
    ZombieProcess(u32),

    #[error("Daemon is connected but did not respond to Ping")]
    DaemonNotResponding,
}
