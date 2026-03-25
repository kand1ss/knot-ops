use crate::{
    messages::MessageContext,
    transport::{RawTransport, TransportSpec},
};
use knot_core::errors::TransportError;

/// A trait abstraction for asynchronous message handlers.
///
/// `AsyncHandler` is a specialized helper trait designed to bridge the gap between
/// asynchronous closures and Rust's lifetime system. It allows the transport
/// to execute handlers that borrow data from the [`MessageContext`] without
/// requiring the future to be `'static`.
///
/// ### Why this exists
/// In standard Rust, returning a `Future` from a closure that borrows its
/// arguments often leads to "lifetime may not live long enough" errors.
/// `AsyncHandler` uses **Higher-Rank Trait Bounds (HRTB)** to ensure that
/// for every lifetime `'a`, there is a corresponding `Future` type that
/// is valid for that specific duration.
///
/// ### Characteristics
/// - **Zero-Cost**: Unlike `BoxFuture`, this trait preserves static dispatch
///   and avoids unnecessary heap allocations.
/// - **Thread-Safe**: Requires `Send + Sync` to ensure compatibility with
///   multithreaded executors like Tokio.
pub trait AsyncHandler<'a, R, S>: Send + Sync
where
    R: RawTransport,
    S: TransportSpec,
{
    /// The specific [`Future`] type returned by the handler.
    /// It is constrained by the lifetime `'a` to match the incoming context.
    type Fut: Future<Output = Result<(), TransportError>> + Send + 'a;

    /// Invokes the handler with the provided message context.
    fn call(&self, ctx: MessageContext<'a, R, S>) -> Self::Fut;
}

/// Blanket implementation for any closure that returns a compatible Future.
///
/// This allows standard closures (e.g., `async |ctx| { ... }`) to be
/// passed directly into transport methods like `serve_with`.
impl<'a, R, S, F, Fut> AsyncHandler<'a, R, S> for F
where
    R: RawTransport,
    S: TransportSpec,
    F: Fn(MessageContext<'a, R, S>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<(), TransportError>> + Send + 'a,
{
    type Fut = Fut;

    #[inline]
    fn call(&self, ctx: MessageContext<'a, R, S>) -> Self::Fut {
        (self)(ctx)
    }
}
