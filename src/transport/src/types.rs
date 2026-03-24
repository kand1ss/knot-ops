//! # Knot Protocol Types
//!
//! This module defines high-level type aliases to simplify the usage of
//! the transport layer with the standard Knot communication protocol.

use crate::{
    codec::BinaryCodec,
    messages::daemon::{DaemonEvent, DaemonRequest, DaemonResponse},
    transport::{MessageTransport, TransportSpec, ipc::IpcTransport},
};

/// The default protocol specification for the Knot Daemon.
///
/// This structure implements [`TransportSpec`] to define the standard
/// interaction patterns between the CLI and the background process.
/// It binds together the command set, response types, and the binary
/// serialization format used in production.
pub struct DaemonTransportSpec;
impl TransportSpec for DaemonTransportSpec {
    type Req = DaemonRequest;
    type Res = DaemonResponse;
    type Ev = DaemonEvent;
    type C = BinaryCodec;
}

/// A specialized [`MessageTransport`] pre-configured for the Knot protocol.
///
/// This type alias simplifies the creation of IPC channels over Unix Domain Sockets.
/// It uses the [`IpcTransport`] as the I/O layer and [`DaemonTransportSpec`]
/// to enforce type-safe communication.
pub type DaemonTransport = MessageTransport<IpcTransport, DaemonTransportSpec>;
