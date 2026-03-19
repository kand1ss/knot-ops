use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("failed to connect to daemon socket '{path}': {source}")]
    ConnectionFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("message is too large")]
    MessageTooLarge { size: usize },

    #[error("client didn't expect a message")]
    UnexpectedMessage,

    #[error("daemon is not running (socket exists but connection refused)")]
    ConnectionRefused,

    #[error("daemon is not running (socket not found at '{path}')")]
    InvalidSocketPath { path: PathBuf },

    #[error("request timed out after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("failed to serialize message: {reason}")]
    SerializeError { reason: String },

    #[error("failed to deserialize message: {reason}")]
    DeserializeError { reason: String },

    #[error("connection closed unexpectedly")]
    ConnectionClosed,

    #[error("transport io error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
}
