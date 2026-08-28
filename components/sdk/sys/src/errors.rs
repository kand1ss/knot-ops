use std::io;
use thiserror::Error;

/// Represents errors that can occur during a process-related operation.
///
/// # Variants
///
/// * `NotRunning`: Indicates that the process is not currently running.
///
/// * `Mismatch { expected, actual }`: Indicates that there is a mismatch
///   in the process name. It provides details about the expected process
///   name and the actual process name encountered during the operation.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process is not running")]
    NotRunning,

    #[error("process name mismatch: expected '{expected}', got '{actual}'")]
    Mismatch { expected: String, actual: String },

    #[error("process reused")]
    Reused,

    #[error("io error")]
    Io(#[from] io::Error),
}
