use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("failed to connect to socket '{path}': {source}")]
    ConnectionFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("message size ({size} bytes) exceeds limit")]
    MessageTooLarge { size: usize },

    #[error("received a message that was not expected in the current state")]
    UnexpectedMessage,

    #[error("unexpected message kind: expected {expected}, found {found}")]
    UnexpectedKind { expected: String, found: String },

    #[error("connection refused (target may not be listening)")]
    ConnectionRefused,

    #[error("invalid socket path: '{path}'")]
    InvalidSocketPath { path: PathBuf },

    #[error("invalid metadata: '{metadata}'")]
    InvalidMetadata { metadata: String },

    #[error("operation timed out after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("serialization failed: {reason}")]
    SerializeError { reason: String },

    #[error("deserialization failed: {reason}")]
    DeserializeError { reason: String },

    #[error("middleware '{name}' rejected the message: {reason}")]
    MiddlewareBlocked { name: String, reason: String },

    #[error("underlying connection was closed unexpectedly")]
    ConnectionClosed,

    #[error("transport I/O error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
}
impl TransportError {
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::ConnectionFailed { .. }
                | Self::ConnectionRefused { .. }
                | Self::InvalidSocketPath { .. }
                | Self::ConnectionClosed
        )
    }
}
