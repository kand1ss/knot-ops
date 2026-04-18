use crate::{
    messages::{MAX_METADATA_KEY_LEN, MAX_METADATA_VALUE_LEN, Message, MessageKind, MetadataMap},
    transport::{MessageTransport, RawTransport, TransportSpec},
};
use knot_core::errors::TransportError;
use std::{
    borrow::Cow,
    fmt::Debug,
    ops::{Deref, DerefMut},
};
use tracing::{debug, error, info, instrument, warn};

/// A high-level wrapper around an incoming message and its associated transport.
///
/// `MessageContext` tracks whether a response has been sent and automatically
/// handles correlation IDs, ensuring that replies are correctly routed back
/// to the requester.
///
/// ### Lifetime
/// * `'a`: The lifetime of the reference to the underlying [`MessageTransport`].
#[derive(Debug)]
pub struct MessageContext<'a, R, S>
where
    R: RawTransport + 'static,
    S: TransportSpec,
{
    /// A reference to the transport that received the message.
    transport: &'a MessageTransport<R, S>,
    /// The actual received message envelope.
    message: Message<S::Req, S::Res, S::Ev>,
    /// Internal flag to prevent or warn about multiple responses to the same request.
    replied: bool,
    /// Optional metadata map that accumulates outgoing metadata for replies or events.
    outgoing_metadata: Option<MetadataMap>,
}

/// A type alias for deconstructing a context into its raw components.
///
/// Useful when the ownership of the message is required.
pub type MessageContextParts<'a, R, S> = (
    Message<<S as TransportSpec>::Req, <S as TransportSpec>::Res, <S as TransportSpec>::Ev>,
    &'a MessageTransport<R, S>,
);

impl<'a, R, S> MessageContext<'a, R, S>
where
    R: RawTransport + 'static,
    S: TransportSpec,
{
    /// Creates a new `MessageContext` from a message and a transport reference.
    pub fn new(
        message: Message<S::Req, S::Res, S::Ev>,
        transport: &'a MessageTransport<R, S>,
    ) -> Self {
        Self {
            transport,
            message,
            replied: false,
            outgoing_metadata: None,
        }
    }

    /// Sets a metadata entry for any outgoing replies or events generated from this context.
    ///
    /// This metadata is stored separately from the incoming request's metadata, ensuring
    /// that only explicitly set outgoing metadata is sent back to the client.
    pub fn set_meta<K, V>(&mut self, key: K, value: V) -> Result<(), TransportError>
    where
        K: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        let key_str = key.into();
        let val_str = value.into();
        Message::<S::Req, S::Res, S::Ev>::validate_metadata(&key_str, MAX_METADATA_KEY_LEN)?;
        Message::<S::Req, S::Res, S::Ev>::validate_metadata(&val_str, MAX_METADATA_VALUE_LEN)?;

        let map = self.outgoing_metadata.get_or_insert_with(MetadataMap::new);
        map.insert_str(key_str, val_str);
        Ok(())
    }

    /// Sends a response back to the client.
    ///
    /// This method automatically uses the `id` of the original request for correlation.
    /// It also tracks state to prevent accidental duplicate responses.
    ///
    /// # Warning
    /// If called more than once, a warning will be logged to `stderr` indicating
    /// a potential logic error in the request handler.
    #[instrument(
        skip(self, msg),
        fields(
            msg_id = %self.message.id,
            re_reply = self.replied
        ),
        name = "context_reply"
    )]
    pub async fn reply(&mut self, msg: S::Res) -> Result<(), TransportError> {
        if self.replied {
            warn!(
                request_id = %self.message.id,
                "Logic error: attempted to reply twice to the same request"
            );
        }

        let message = Message::response(self.message.id, msg)
            .maybe_with_metadata(self.outgoing_metadata.clone());
        debug!("Sending response back to client...");

        match self.transport.send(message).await {
            Ok(_) => {
                self.replied = true;
                info!("Successfully sent reply to client");
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Failed to send reply to client");
                Err(e)
            }
        }
    }

    /// Emits an arbitrary message (e.g., an Event) through the transport.
    ///
    /// Unlike `reply`, this does not affect the `replied` state and does not
    /// automatically set correlation IDs.
    #[instrument(
        skip(self, msg),
        fields(msg_kind = ?msg.kind),
        name = "context_emit"
    )]
    pub async fn emit(&self, msg: Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        debug!("Emitting arbitrary message...");

        let msg = msg.maybe_with_metadata(self.outgoing_metadata.clone());

        if let Err(e) = self.transport.send(msg).await {
            error!(error = %e, "Failed to emit message");
            return Err(e);
        }

        info!("Message emitted successfully");
        Ok(())
    }

    /// Sends an event associated with the current request back to the client.
    ///
    /// This method automatically uses the `id` of the original request to link
    /// the event to the ongoing request/response cycle. This is useful for
    /// streaming partial updates or progress before a final `reply` is sent.
    ///
    /// Unlike `reply`, sending an event does not mark the request as `replied`,
    /// allowing you to send multiple events during the lifecycle of a request.
    #[instrument(skip(self), name = "context_event")]
    pub async fn event(&self, event: S::Ev) -> Result<(), TransportError> {
        let message = Message::event(self.message.id, event)
            .maybe_with_metadata(self.outgoing_metadata.clone());
        debug!("Sending event back to client...");

        match self.transport.send(message).await {
            Ok(_) => {
                info!("Successfully sent event to client");
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Failed to send event to client");
                Err(e)
            }
        }
    }

    /// Returns a reference to the encapsulated message.
    pub fn get(&self) -> &Message<S::Req, S::Res, S::Ev> {
        &self.message
    }

    /// Returns a reference to the message kind (Request, Response, or Event).
    pub fn kind(&self) -> &MessageKind<S::Req, S::Res, S::Ev> {
        &self.message.kind
    }

    /// Consumes the context and returns the original message and transport reference.
    pub fn into_parts(self) -> MessageContextParts<'a, R, S> {
        (self.message, self.transport)
    }
}

impl<'a, R, S> Deref for MessageContext<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    type Target = Message<S::Req, S::Res, S::Ev>;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl<'a, R, S> DerefMut for MessageContext<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.message
    }
}
