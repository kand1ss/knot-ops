use crate::config::ServiceConfig;

/// The specific reason why a process has terminated.
pub enum ExitReason {
    /// The process was terminated by an explicit user command (e.g., a stop signal).
    UserRequest,
    /// The process terminated unexpectedly due to a crash or internal error.
    Faulted,
}

/// Events emitted during the lifecycle of a process.
///
/// Used for monitoring, logging, and triggering dependency-based actions.
pub enum ProcessEvent<'a> {
    /// Emitted when a process is successfully launched.
    Started {
        /// The PID of the newly started process.
        pid: &'a str,
        /// Reference to the configuration used to start the service.
        service: &'a ServiceConfig,
    },
    /// Emitted when a process is gracefully shut down.
    Stopped {
        pid: &'a str,
        service: &'a ServiceConfig,
    },
    /// Emitted when a process fails to start or crashes during execution.
    Failed {
        pid: &'a str,
        service: &'a ServiceConfig,
    },
    /// Emitted when a process finishes execution and closes.
    Exited {
        pid: &'a str,
        service: &'a ServiceConfig,
        /// The reason behind the process termination.
        reason: ExitReason,
    },
}
