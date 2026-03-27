//! # Knot Daemon Protocol
//!
//! This module defines the high-level API for the Knot daemon.
//!
//! It includes request/response structures for managing process lifecycles,
//! querying service health, and retrieving system status.

use crate::messages::Message;
use knot_core::data::ServiceData;
use knot_core::states::ServiceStatus;
use knot_core::utils::TimestampUtils;
use serde::{Deserialize, Serialize};

/// A serialized snapshot of a service's current state.
///
/// This structure is sent from the daemon to the CLI to provide
/// human-readable information about a managed process.
#[derive(Serialize, Deserialize, Debug)]
pub struct ServiceStatusResponse {
    /// Process ID (PID) assigned by the operating system.
    pub pid: u32,
    /// Unique identifier for the service.
    pub name: String,
    /// String representation of the service lifecycle state.
    pub status: String,
    /// Formatted duration (e.g., "2h 15m") since the service started.
    pub uptime: String,
    /// Simple boolean indicating if the service is currently operational.
    pub healthy: bool,
}

impl From<&ServiceData> for ServiceStatusResponse {
    /// Converts internal `ServiceData` into a serializable `ServiceStatusResponse`.
    ///
    /// This handles the conversion of raw timestamps into formatted uptime strings
    /// and determines the health status based on the `ServiceStatus` enum.
    fn from(s: &ServiceData) -> Self {
        Self {
            pid: s.pid,
            name: s.name.clone(),
            status: s.status.to_string(),
            uptime: TimestampUtils::format_uptime(s.timestamp),
            healthy: s.status == ServiceStatus::Running,
        }
    }
}

/// Commands sent from the CLI to the Knot Daemon.
#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonRequest {
    /// Request to gracefully shut down the daemon and all managed services.
    Down,
    /// Request to retrieve the status of all currently registered services.
    Status,
}

/// Information sent from the Knot Daemon back to the CLI.
#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonResponse {
    /// Indicates that the requested operation was received.
    Ok,
    /// Indicates a failure occurred during the operation.
    Error {
        /// Human-readable explanation of what went wrong.
        message: String,
    },
    /// Contains a list of service snapshots in response to a `Status` request.
    Status {
        /// A vector of individual service statuses.
        services: Vec<ServiceStatusResponse>,
    },
}

/// Represents asynchronous notifications sent from the Daemon to connected clients.
///
/// Unlike responses, events are unprompted and used to broadcast state changes,
/// real-time logs, or lifecycle updates across the system.
#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonEvent {
    /// A generic event related to a specific service managed by the orchestrator.
    ///
    /// This can include status transitions (e.g., from 'Starting' to 'Running')
    ServiceEvent(ServiceStatusResponse),
}

/// Concrete type alias for the Knot message protocol.
///
/// This combines the generic `Message` envelope with Knot-specific
/// requests and responses.
pub type DaemonMessage = Message<DaemonRequest, DaemonResponse, DaemonEvent>;
