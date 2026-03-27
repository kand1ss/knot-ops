use crate::{
    messages::Message,
    middleware::traits::Middleware,
    transport::{RawTransport, TransportSpec},
};
use knot_core::errors::TransportError;

/// A collection of middlewares organized into an executable chain.
///
/// The `Pipeline` is responsible for managing the registration of [`Middleware`]
/// implementations and orchestrating their execution in a specific order.
/// It uses an index-based recursion to move messages from one layer to the next.
///
/// # Concurrency
/// Since `Pipeline` is often wrapped in an `Arc` or protected by a lock (like `RwLock`)
/// within the transport, it is designed to be `Send + Sync`.
#[derive(Debug)]
pub struct Pipeline<R: RawTransport, S: TransportSpec> {
    /// Internal storage for boxed middleware trait objects.
    pub(crate) middlewares: Vec<Box<dyn Middleware<R, S>>>,
}

impl<R, S> Pipeline<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// Adds a middleware to the end of the pipeline.
    ///
    /// Middlewares are executed in the order they are added (FIFO).
    pub fn add_middleware<M: Middleware<R, S>>(&mut self, middleware: M) {
        self.middlewares.push(Box::new(middleware));
    }

    /// Initiates the inbound middleware processing for a received message.
    ///
    /// This is the primary entry point for processing data coming from the transport.
    /// It starts the recursive execution from the first middleware (index 0).
    ///
    /// # Errors
    /// Returns a [`TransportError`] if any middleware in the chain fails or
    /// explicitly blocks the message.
    pub async fn execute_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        self.invoke_recv(0, msg).await
    }

    /// Initiates the outbound middleware processing for a message being sent.
    ///
    /// This method ensures that all outgoing data is intercepted by the middleware
    /// chain (e.g., for adding metadata or encryption) before reaching the transport.
    ///
    /// # Errors
    /// Returns a [`TransportError`] if the chain execution is interrupted.
    pub async fn execute_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        self.invoke_send(0, msg).await
    }

    /// Creates a state object for the next middleware in the chain.
    ///
    /// This internal helper tracks the current position in the `middlewares` vector
    /// to ensure the message progresses linearly.
    async fn update_state(&self, index: usize) -> NextState<'_, R, S> {
        NextState {
            pipeline: self,
            next_index: index + 1,
        }
    }

    /// Recursively invokes the inbound middleware at the specified index.
    ///
    /// If the index is out of bounds, it signifies the end of the pipeline,
    /// and the processing is considered successful (`Ok(())`).
    ///
    /// The current middleware receives an [`Inbound`] handle, which it must call
    /// to continue the chain.
    async fn invoke_recv(
        &self,
        index: usize,
        msg: &Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        let Some(mw) = self.middlewares.get(index) else {
            return Ok(());
        };

        let mut state = self.update_state(index).await;
        mw.on_recv(msg, Inbound(&mut state)).await
    }

    /// Recursively invokes the outbound middleware at the specified index.
    ///
    /// Similar to `invoke_recv`, but operates on outgoing messages using the
    /// [`Outbound`] handle.
    async fn invoke_send(
        &self,
        index: usize,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        let Some(mw) = self.middlewares.get(index) else {
            return Ok(());
        };

        let mut state = self.update_state(index).await;
        mw.on_send(msg, Outbound(&mut state)).await
    }
}
impl<R, S> Default for Pipeline<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    fn default() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }
}

/// A handle representing the remaining part of the inbound middleware chain.
///
/// Passed to [`Middleware::on_recv`], it allows the current layer to delegate
/// processing to the next middleware in the pipeline.
pub struct Inbound<'a, R: RawTransport, S: TransportSpec>(&'a mut NextState<'a, R, S>);

/// A handle representing the remaining part of the outbound middleware chain.
///
/// Passed to [`Middleware::on_send`], it serves as a continuation for outgoing
/// messages, moving them further down the stack towards the transport.
pub struct Outbound<'a, R: RawTransport, S: TransportSpec>(&'a mut NextState<'a, R, S>);

/// An internal "cursor" that tracks the progression of a message through the [`Pipeline`].
///
/// This structure maintains the current position within the middleware vector
/// and provides a reference back to the pipeline itself. It is designed to be
/// short-lived, existing only for the duration of a single message's journey.
///
/// # Lifetimes
/// * `'a`: Ties the state to the parent [`Pipeline`] to prevent dangling references.
struct NextState<'a, R: RawTransport, S: TransportSpec> {
    /// The index of the middleware to be executed next.
    next_index: usize,
    /// A reference to the parent pipeline containing the middleware vector.
    pipeline: &'a Pipeline<R, S>,
}

impl<'a, R, S> Inbound<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    pub async fn run(self, msg: &Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        let next_index = self.0.next_index;
        let pipeline = self.0.pipeline;

        if let Some(mw) = pipeline.middlewares.get(next_index) {
            self.0.next_index += 1;
            mw.on_recv(msg, self).await
        } else {
            Ok(())
        }
    }
}

impl<'a, R, S> Outbound<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    pub async fn run(self, msg: &mut Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        let next_index = self.0.next_index;
        let pipeline = self.0.pipeline;

        if let Some(mw) = pipeline.middlewares.get(next_index) {
            self.0.next_index += 1;
            mw.on_send(msg, self).await
        } else {
            Ok(())
        }
    }
}
