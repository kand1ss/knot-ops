//! Defines the core abstractions for the Knot middleware system.
//!
//! This module contains the [`Middleware`] trait, which is the fundamental
//! building block for extending the transport's behavior. By implementing
//! this trait, developers can hook into the message processing lifecycle
//! to provide cross-cutting concerns like logging, validation, or security.
use crate::{
    messages::Message,
    middleware::{Inbound, Outbound},
    transport::{RawTransport, TransportSpec},
};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use std::fmt::Debug;

/// A trait for implementing asynchronous middleware layers for the Knot transport.
///
/// Middlewares allow for intercepting, logging, or rejecting messages
/// as they flow through the transport pipeline. This follows the Chain of
/// Responsibility pattern, where each middleware can perform actions before
/// and after the next layer in the chain.
///
/// # Requirements
/// - Must be `Send + Sync` to safely move across thread boundaries in an async context.
/// - Must be `'static` to ensure it can be stored within the transport's long-lived pipeline.
/// - Must implement `Debug` for easier troubleshooting and system introspection.
#[async_trait]
pub trait Middleware<R: RawTransport, S: TransportSpec>: Send + Sync + Debug + 'static {
    /// Intercepts and processes incoming messages.
    ///
    /// This method is called when a message is received from the transport layer,
    /// before it reaches the final application logic (Daemon or CLI handler).
    ///
    /// # Parameters
    /// - `msg`: A read-only reference to the incoming [`Message`].
    /// - `next`: A handle to the next stage in the **inbound** pipeline.
    ///
    /// # Execution Flow
    /// To pass the message further down the pipeline, the implementation **must** /// call `next.run(msg).await`. Omitting this call "short-circuits" the flow,
    /// effectively dropping the message.
    ///
    /// # Errors
    /// Returns [`TransportError`] to halt the pipeline. If an error is returned,
    /// the message is not processed further, and the error is propagated
    /// to the transport's receiver.
    ///
    /// # Example
    /// ```rust,ignore
    /// async fn on_recv(&self, msg: &Message<S>, next: Inbound<'_, R, S>) -> Result<(), TransportError> {
    ///     println!("Inbound message: {:?}", msg.kind());
    ///     next.run(msg).await
    /// }
    /// ```
    async fn on_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
        next: Inbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        next.run(msg).await
    }

    /// Intercepts and potentially modifies outgoing messages.
    ///
    /// This method is called before a message is serialized and sent over the
    /// transport. Unlike `on_recv`, this method provides a mutable reference,
    /// allowing the middleware to transform the message or its metadata.
    ///
    /// # Parameters
    /// - `msg`: A mutable reference to the outgoing [`Message`].
    /// - `next`: A handle to the next stage in the **outbound** pipeline.
    ///
    /// # Key Capabilities
    /// - **Transformation**: Modify the payload or headers before transmission.
    /// - **Metadata Injection**: Add tracing IDs, timestamps, or system-level flags.
    /// - **Filtering**: Prevent specific messages from being sent by returning an error.
    ///
    /// # Execution Flow
    /// The implementation **must** call `next.run(msg).await` to allow the
    /// message to proceed to the transport's sender.
    async fn on_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
        next: Outbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        next.run(msg).await
    }
}
