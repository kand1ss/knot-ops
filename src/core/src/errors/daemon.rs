use thiserror::Error;
use std::path::PathBuf;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("daemon already running (pid={pid})")]
    AlreadyRunning { pid: u32 },

    #[error("daemon crashed (stale pid file found)")]
    Crashed,

    #[error("daemon is not running")]
    NotRunning,

    #[error("daemon failed to start within {seconds}s")]
    StartTimeout { seconds: u64 },

    #[error("failed to write pid file '{path}': {source}")]
    PidFileError {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to create socket '{path}': {source}")]
    SocketError {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("state store error: {reason}")]
    StateStoreError { reason: String },

    #[error("failed to recover from crash: {reason}")]
    RecoveryFailed { reason: String },
}