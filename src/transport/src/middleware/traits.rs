//! Defines the core abstractions for the Knot middleware system.
//!
//! This module contains the [`Middleware`] trait, which is the fundamental
//! building block for extending the transport's behavior. By implementing
//! this trait, developers can hook into the message processing lifecycle
//! to provide cross-cutting concerns like logging, validation, or security.
use crate::{
    messages::MessageContext,
    middleware::Next,
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
    /// Handles an incoming message context and controls the execution of the next layer.
    ///
    /// # Parameters
    /// - `ctx`: A reference to the [`MessageContext`], providing access to the
    ///   received message and transport capabilities.
    /// - `next`: A handle to the next middleware in the pipeline.
    ///
    /// # Execution Flow
    /// To allow the message to continue to the next middleware or the final
    /// application logic, the implementation **must** call `next.run(ctx).await`.
    ///
    /// If `next.run(ctx).await` is not called, the pipeline is "short-circuited,"
    /// and the message will be dropped or treated as handled by this layer.
    ///
    /// # Errors
    /// Returns [`TransportError`] if the middleware fails. If an error is returned,
    /// the pipeline execution stops immediately, and the error is propagated
    /// to the transport's `recv` caller.
    ///
    /// # Example
    /// ```rust,ignore
    /// #[async_trait]
    /// impl<R, S> Middleware<R, S> for LoggerMiddleware {
    ///     async fn handle(&self, ctx: &MessageContext<'_, R, S>, next: Next<'_, R, S>) -> Result<(), TransportError> {
    ///         println!("Request received: {:?}", ctx.kind());
    ///         
    ///         // Continue to the next layer
    ///         let result = next.run(ctx).await;
    ///         
    ///         println!("Request processed.");
    ///         result
    ///     }
    /// }
    /// ```
    async fn handle(
        &self,
        ctx: &MessageContext<'_, R, S>,
        next: Next<'_, R, S>,
    ) -> Result<(), TransportError>;
}
