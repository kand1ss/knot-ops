use strum::{Display, EnumString};

/// Defines the possible lifecycle states of a managed service.
#[derive(Display, EnumString, Debug, PartialEq)]
pub enum ServiceStatus {
    /// The service is not currently running.
    Stopped,
    /// The service is in the process of initializing or launching.
    Starting,
    /// The service is active and operating normally.
    Running,
    /// The service encountered an error and is no longer operational.
    Failed,
}
