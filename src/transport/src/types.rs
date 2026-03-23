//! # Knot Protocol Types
//!
//! This module defines high-level type aliases to simplify the usage of 
//! the transport layer with the standard Knot communication protocol.

use crate::{
    transport::MessageTransport,
    messages::daemon::{DaemonRequest, DaemonResponse, DaemonEvent}
};

/// A specialized [`MessageTransport`] for the Knot Daemon protocol.
///
/// This alias pre-configures the transport with:
/// - **Requests**: [`DaemonRequest`]
/// - **Responses**: [`DaemonResponse`]
/// - **Events**: [`DaemonEvent`]
///
/// It still allows flexibility in choosing the underlying I/O [`Transport`] 
/// (e.g., `IpcTransport`) and the serialization [`Codec`].
pub type DaemonTransport<Transport, Codec> 
    = MessageTransport<Transport, DaemonRequest, DaemonResponse, DaemonEvent, Codec>;