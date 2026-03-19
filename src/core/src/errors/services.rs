use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("service '{name}' not found")]
    NotFound { name: String },

    #[error("service '{name}' is already running (pid={pid})")]
    AlreadyRunning { name: String, pid: u32 },

    #[error("service '{name}' is not running")]
    NotRunning { name: String },

    #[error("failed to start service '{name}': {reason}")]
    StartFailed { name: String, reason: String },

    #[error("failed to stop service '{name}': {reason}")]
    StopFailed { name: String, reason: String },

    #[error("service '{name}' failed to become healthy within {seconds}s")]
    HealthCheckTimeout { name: String, seconds: u64 },

    #[error("service '{name}' failed after {attempts} restart attempts")]
    RestartLimitExceeded { name: String, attempts: u32 },

    #[error("service '{name}' cannot start: dependency '{dependency}' is {status}")]
    DependencyNotReady {
        name: String,
        dependency: String,
        status: String,
    },

    #[error("group '{name}' not found")]
    GroupNotFound { name: String },
}
