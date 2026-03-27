use crate::{
    messages::{Message, MessageKind, MetadataMap},
    transport::{MessageTransport, RawTransport, TransportSpec},
};
use knot_core::errors::TransportError;
use std::ops::{Deref, DerefMut};

/// A high-level wrapper around an incoming message and its associated transport.
///
/// `MessageContext` tracks whether a response has been sent and automatically
/// handles correlation IDs, ensuring that replies are correctly routed back
/// to the requester.
///
/// ### Lifetime
/// * `'a`: The lifetime of the reference to the underlying [`MessageTransport`].
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
        }
    }

    /// Sends a response back to the client.
    ///
    /// This method automatically uses the `id` of the original request for correlation.
    /// It also tracks state to prevent accidental duplicate responses.
    ///
    /// # Warning
    /// If called more than once, a warning will be logged to `stderr` indicating
    /// a potential logic error in the request handler.
    pub async fn reply(
        &mut self,
        msg: S::Res,
        metadata: Option<MetadataMap>,
    ) -> Result<(), TransportError> {
        if !self.replied {
            eprintln!(
                "WARNING: MessageContext replied twice to request ID {}",
                self.message.id
            );
        }

        let message = Message::response(self.message.id, msg).maybe_with_metadata(metadata);

        self.replied = true;
        self.transport.send(message).await
    }

    /// Emits an arbitrary message (e.g., an Event) through the transport.
    ///
    /// Unlike `reply`, this does not affect the `replied` state and does not
    /// automatically set correlation IDs.
    pub async fn emit(&self, msg: Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        self.transport.send(msg).await
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
