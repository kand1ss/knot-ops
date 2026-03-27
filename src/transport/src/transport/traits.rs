//! # Knot Transport Abstractions
//!
//! This module defines the core traits that decouple the messaging logic from
//! the underlying I/O implementation (e.g., Unix Domain Sockets, TCP, or Mock).
//!
//! By using these traits, the Knot orchestrator can remain agnostic about the
//! specific networking layer while maintaining strict type safety for its protocol.
use crate::{
    codec::MessageCodec,
    transport::{MAX_MESSAGE_SIZE, MessageTransport},
};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

/// Defines the type requirements for a specific communication protocol.
///
/// `TransportSpec` acts as a "trait bundle" that links a specific set of
/// Request, Response, and Event types with a compatible Codec.
/// This prevents type mismatches when setting up a `MessageTransport`.
pub trait TransportSpec: Send + Sync + 'static {
    /// The type of request messages handled by this specification.
    type Req: Serialize + DeserializeOwned + Send + Sync + Debug + 'static;
    /// The type of response messages handled by this specification.
    type Res: Serialize + DeserializeOwned + Send + Sync + Debug + 'static;
    /// The type of asynchronous events handled by this specification.
    type Ev: Serialize + DeserializeOwned + Send + Sync + Debug + 'static;
    /// The codec responsible for serializing/deserializing these types.
    type C: MessageCodec<Raw = Vec<u8>>;
}

/// Defines a low-level byte-oriented transport layer.
///
/// This trait serves as the foundation for all communication in Knot. It abstracts away
/// the details of the underlying I/O (IPC, TCP, etc.), focusing strictly on sending
/// and receiving discrete byte frames.
///
/// ### Frame Integrity
/// Implementors must guarantee that a single call to [`Self::recv_frame`] returns
/// exactly one complete message frame. Fragmentation and reassembly must be handled
/// internally by the implementor.
#[async_trait]
pub trait RawTransport: Send + Sync + Sized + 'static {
    /// Validates the size of a frame against [`MAX_MESSAGE_SIZE`].
    ///
    /// This is called automatically by the default implementations of
    /// `send_frame` and `recv_frame`.
    fn check_frame_size(frame: &[u8]) -> Result<(), TransportError> {
        let len = frame.len();
        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge { size: len });
        }
        Ok(())
    }

    /// Sends a raw byte slice as a single atomic frame.
    ///
    /// This method performs a size check before calling the internal implementation.
    /// It ensures that no oversized frames are sent into the network.
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
        Self::check_frame_size(frame)?;
        self.send_frame_internal(frame).await
    }

    /// The underlying implementation for sending bytes.
    ///
    /// Must be implemented by specific transports (e.g., `IpcTransport`).
    async fn send_frame_internal<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError>;

    /// Receives a single complete byte frame from the transport.
    ///
    /// This method awaits a full frame from the internal implementation and
    /// validates its size before returning. It is the primary way to read raw
    /// data from the stream.
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError> {
        let frame: Vec<u8> = self.recv_frame_internal().await?;
        Self::check_frame_size(&frame)?;
        Ok(frame)
    }

    /// The underlying implementation for receiving bytes.
    ///
    /// Should block or await until a full frame is available or the connection is closed.
    async fn recv_frame_internal(&self) -> Result<Vec<u8>, TransportError>;

    /// Upcasts the raw byte transport into a high-level [`MessageTransport`].
    ///
    /// This is the standard entry point for converting a raw I/O stream into
    /// a typed RPC channel using a specific [`TransportSpec`].
    fn to_messaged<S: TransportSpec>(self) -> MessageTransport<Self, S> {
        MessageTransport::new(self)
    }
}

/// Interface for a network-based server that can accept new connections.
///
/// A `Server` implementation manages the lifecycle of a listener (like a
/// `UnixListener`) and produces `RawTransport` instances for each
/// incoming client.
#[async_trait]
pub trait Server {
    /// The address type used by the server (e.g., `PathBuf` for UDS or `SocketAddr` for TCP).
    type Address: Send;
    /// The type of raw transport produced upon accepting a connection.
    type Transport: RawTransport;

    /// Binds to the specified address and starts the listener.
    ///
    /// # Errors
    /// Returns `TransportError` if the address is already in use or
    /// if there are insufficient permissions.
    async fn bind(addr: Self::Address) -> Result<Self, TransportError>
    where
        Self: Sized;

    /// Gracefully stops the server listener.
    ///
    /// This should prevent new connections from being accepted but
    /// does not necessarily drop existing active transports.
    async fn shutdown(&mut self);

    /// Accepts the next incoming connection.
    ///
    /// On success, returns a `RawTransport` that can be converted
    /// into a `MessageTransport` using `.to_messaged()`.
    async fn accept(&self) -> Result<Self::Transport, TransportError>;
}
