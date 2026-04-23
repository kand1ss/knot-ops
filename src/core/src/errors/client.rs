use crate::errors::TransportError;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("daemon is not running (socket not found at {path})")]
    DaemonNotRunning { path: PathBuf },

    #[error("unexpected response type from daemon (expected: {expected})")]
    UnexpectedResponse { expected: String },

    #[error("failed command execution")]
    CommandFailed(String),

    #[error(transparent)]
    Transport(#[from] TransportError),
}
