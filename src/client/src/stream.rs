use knot_core::errors::{ClientError, TransportError};
use knot_transport::{
    messages::MessageKind,
    transport::{MessageTransport, RawTransport, TransportSpec},
};
use std::sync::Arc;
use tracing::instrument;

/// A stream of events received from the daemon.
///
/// This struct wraps a `MessageTransport` and provides a high-level interface
/// for asynchronously receiving events.
pub struct EventStream<R: RawTransport, S: TransportSpec> {
    transport: Arc<MessageTransport<R, S>>,
}

impl<R: RawTransport, S: TransportSpec> EventStream<R, S> {
    /// Creates a new `EventStream` from the given transport.
    pub fn new(transport: Arc<MessageTransport<R, S>>) -> Self {
        Self { transport }
    }

    /// Returns the next event from the stream.
    ///
    /// Returns `Ok(Some(event))` if an event is received, `Ok(None)` if the connection
    /// is closed, or an error if receiving fails.
    #[instrument(skip(self), name = "stream_next")]
    pub async fn next(&self) -> Result<Option<S::Ev>, ClientError> {
        match self.transport.recv().await {
            Ok(msg) => {
                let (message, _) = msg.into_parts();
                match message.kind {
                    MessageKind::Event(ev) => Ok(Some(ev)),
                    _ => Ok(None),
                }
            }
            Err(TransportError::ConnectionClosed) => Ok(None),
            Err(e) => Err(ClientError::Transport(e)),
        }
    }
}
