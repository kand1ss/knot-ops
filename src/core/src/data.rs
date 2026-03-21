use crate::states::ServiceStatus;

/// Represents the current runtime state of a service.
///
/// This structure provides a snapshot of a service's execution details
/// at a specific point in time.
pub struct ServiceData {
    /// The unique Process Identifier (PID) assigned by the operating system.
    pub pid: u32,
    /// The human-readable name of the service.
    pub name: String,
    /// The current operational status of the service.
    pub status: ServiceStatus,
    /// Unix timestamp indicating when this data was last updated.
    pub timestamp: u64,
}
