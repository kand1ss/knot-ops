//! # Knot Transport Abstractions
//!
//! This module defines the core traits that decouple the messaging logic from
//! the underlying I/O implementation (e.g., Unix Domain Sockets, TCP, or Mock).
//!
//! By using these traits, the Knot orchestrator can remain agnostic about the
//! specific networking layer while maintaining strict type safety for its protocol.
use crate::{codec::MessageCodec, transport::{MessageTransport, MAX_MESSAGE_SIZE}};
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

/// Defines a low-level byte-frame transport.
///
/// Implementors are responsible for ensuring that a single call to `recv_frame`
/// returns exactly one complete message frame.
#[async_trait]
pub trait RawTransport: Send + Sync + Sized + 'static {
    fn check_frame_size<'a>(frame: &'a [u8]) -> Result<(), TransportError> {
        let len = frame.len();
        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge { size: len });
        }
        Ok(())
    }

    /// Sends a raw byte slice as a single frame.
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
        Self::check_frame_size(frame)?;
        self.send_frame_internal(frame).await
    }
    async fn send_frame_internal<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError>;

    /// Receives a single complete byte frame from the transport.
    ///
    /// This method should block or await until a full frame is available.
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError> {
        let frame: Vec<u8> = self.recv_frame_internal().await?;
        Self::check_frame_size(&frame)?;
        Ok(frame)
    }
    async fn recv_frame_internal(&self) -> Result<Vec<u8>, TransportError>;

    /// Wraps the raw transport into a high-level `MessageTransport`.
    ///
    /// This is the primary entry point for converting an I/O stream into
    /// a typed communication channel.
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
