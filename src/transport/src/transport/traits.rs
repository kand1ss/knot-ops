use crate::{codec::MessageCodec, transport::MessageTransport};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

pub trait TransportSpec: Send + Sync + 'static {
    type Req: Serialize + DeserializeOwned + Send + Debug + 'static;
    type Res: Serialize + DeserializeOwned + Send + Debug + 'static;
    type Ev: Serialize + DeserializeOwned + Send + Debug + 'static;
    type C: MessageCodec<Raw = Vec<u8>>;
}

/// Defines a low-level byte-frame transport.
///
/// Implementors are responsible for ensuring that a single call to `recv_frame`
/// returns exactly one complete message frame.
#[async_trait]
pub trait RawTransport: Send + Sync + Sized {
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError>;
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError>;

    /// Wraps the raw transport into a high-level `MessageTransport`.
    fn to_messaged<S: TransportSpec>(self) -> MessageTransport<Self, S> {
        MessageTransport::new(self)
    }
}

/// Interface for a network-based server that can accept new connections.
#[async_trait]
pub trait Server {
    type Address: Send;
    type Transport: RawTransport;

    /// Binds to the specified address and starts the listener.
    async fn bind(addr: Self::Address) -> Result<Self, TransportError>
    where
        Self: Sized;

    /// Stops the server listener.
    async fn shutdown(&mut self);

    /// Accepts the next incoming connection.
    async fn accept(&self) -> Result<Self::Transport, TransportError>;
}
