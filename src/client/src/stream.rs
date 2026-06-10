use knot_core::errors::{ClientError, TransportError};
use knot_transport::{
    messages::{Message, MessageKind},
    transport::TransportSpec,
};
use tokio::sync::broadcast;
use tracing::instrument;

type MessageReceiver<S> = broadcast::Receiver<
    Message<<S as TransportSpec>::Req, <S as TransportSpec>::Res, <S as TransportSpec>::Ev>,
>;

/// An asynchronous stream of events received from the daemon.
///
/// `InboxStream` acts as a specialized receiver that filters incoming messages
/// from the transport and yields only event payloads ([`TransportSpec::Ev`]).
/// It is typically created by calling [`KnotClient::stream`] or as a result
/// of long-running operations like `up` or `down`.
///
/// Under the hood, this struct wraps a [`MessageReceiver`], which is often
/// a subscription to a broadcast channel, allowing multiple consumers to
/// observe daemon activity simultaneously.
pub struct InboxStream<S: TransportSpec> {
    receiver: MessageReceiver<S>,
}

impl<S: TransportSpec> InboxStream<S> {
    /// Creates a new `InboxStream` from the provided [`MessageReceiver`].
    ///
    /// # Arguments
    ///
    /// * `receiver` - The underlying receiver instance used to fetch messages.
    pub fn new(receiver: MessageReceiver<S>) -> Self {
        Self { receiver }
    }

    /// Polls for the next event in the stream.
    ///
    /// This method asynchronously waits for a message to arrive. If the
    /// message is an event, it returns the cloned event payload.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(event))` - A new event was successfully received.
    /// * `Ok(None)` - The stream reached its end (the connection was closed)
    ///   or a non-event message was encountered.
    /// * `Err(ClientError)` - An error occurred during transport, such as
    ///   the connection being refused or lost.
    ///
    /// # Cancellation Safety
    ///
    /// This method is cancellation-safe. If the future is dropped before an
    /// event is received, no messages from the underlying receiver will be lost.
    #[instrument(skip(self), name = "stream_next")]
    pub async fn next(&mut self) -> Result<Option<S::Ev>, ClientError> {
        match self.receiver.recv().await {
            Ok(msg) => match &msg.kind {
                MessageKind::Event(ev) => Ok(Some(ev.clone())),
                _ => Ok(None),
            },
            Err(broadcast::error::RecvError::Closed) => return Ok(None),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                return Err(TransportError::ConnectionRefused.into());
            }
        }
    }
}
