//! # Knot Protocol Types
//!
//! This module defines high-level type aliases to simplify the usage of
//! the transport layer with the standard Knot communication protocol.

use crate::{
    codec::BinaryCodec,
    messages::daemon::{DaemonEvent, DaemonRequest, DaemonResponse},
    transport::{MessageTransport, TransportSpec, ipc::IpcTransport},
};

pub struct DaemonTransportSpec;
impl TransportSpec for DaemonTransportSpec {
    type Req = DaemonRequest;
    type Res = DaemonResponse;
    type Ev = DaemonEvent;
    type C = BinaryCodec;
}

/// A specialized [`MessageTransport`] for the Knot Daemon protocol.
///
/// This alias pre-configures the transport with:
/// - **Requests**: [`DaemonRequest`]
/// - **Responses**: [`DaemonResponse`]
/// - **Events**: [`DaemonEvent`]
///
/// It still allows flexibility in choosing the underlying I/O [`Transport`]
/// (e.g., `IpcTransport`) and the serialization [`Codec`].
pub type DaemonTransport = MessageTransport<IpcTransport, DaemonTransportSpec>;
