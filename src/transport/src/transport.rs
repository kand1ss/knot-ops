//! # Knot Transport
//!
//! This module provides the infrastructure for typed, asynchronous messaging.
//! It handles frame-based I/O, serialization, and request-response synchronization.

use crate::codec::MessageCodec;
use crate::messages::{Message, MessageContext, MessageKind};
use crate::middleware::{Pipeline, traits::Middleware};
use knot_core::errors::TransportError;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::{
    sync::{Mutex, RwLock, mpsc, oneshot},
    time::{Duration, timeout},
};

mod handler;
pub mod ipc;
mod traits;

pub use handler::*;
pub use traits::*;

type PendingResponse<TResponse> = oneshot::Sender<TResponse>;

/// Internal state shared between the public API and the background read loop.
#[derive(Debug)]
pub struct SharedState<S: TransportSpec> {
    /// Tracks requests waiting for a response.
    pub pending: Mutex<HashMap<u32, PendingResponse<S::Res>>>,
    /// Channel to send incoming requests or unhandled responses to the consumer.
    pub inbox_tx: mpsc::Sender<Message<S::Req, S::Res, S::Ev>>,
}

/// A convenient type alias for an incoming message defined by the [`TransportSpec`].
type InboundMessage<S> =
    Message<<S as TransportSpec>::Req, <S as TransportSpec>::Res, <S as TransportSpec>::Ev>;

/// A thread-safe, asynchronous receiver for inbound messages.
///
/// Messages are placed in this "inbox" by a background worker task
/// and consumed by the [`MessageTransport::recv`] method.
type InboxRx<S> = Mutex<mpsc::Receiver<InboundMessage<S>>>;

/// A high-level, asynchronous transport for typed communication.
///
/// `MessageTransport` manages the lifecycle of a connection, including:
/// - **Serialization**: Using the provided [`MessageCodec`].
/// - **Multiplexing**: Handling multiple concurrent requests and background events.
/// - **State Management**: Matching responses to requests using unique IDs.
///
/// It acts as the primary interface for both the Knot CLI and Daemon to
/// send and receive protocol-compliant messages.
#[derive(Debug)]
pub struct MessageTransport<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// The underlying low-level frame transport (e.g., UDS or TCP).
    raw_transport: Arc<R>,
    /// Thread-safe counter for generating unique correlation IDs for outbound requests.
    next_id: AtomicU32,
    /// Shared state between the transport and its background worker (e.g., pending response channels).
    shared: Arc<SharedState<S>>,
    /// The protected receiver for messages that aren't direct responses (Events or Requests).
    inbox_rx: InboxRx<S>,
    /// Zero-cost marker to link the transport to a specific serialization format.
    _phantom: PhantomData<S::C>,
    pipeline: RwLock<Pipeline<R, S>>,
}

impl<R, S> MessageTransport<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// Creates a new `MessageTransport` and spawns a background read loop.
    pub fn new(raw: R) -> Self {
        let (inbox_tx, inbox_rx) = mpsc::channel(32);
        let raw_transport = Arc::new(raw);

        let shared = Arc::new(SharedState {
            pending: Mutex::new(HashMap::new()),
            inbox_tx,
        });

        // Spawn the worker that processes incoming frames
        tokio::spawn(Self::read_loop(
            Arc::clone(&raw_transport),
            Arc::clone(&shared),
        ));

        Self {
            raw_transport,
            next_id: AtomicU32::new(0),
            shared,
            inbox_rx: Mutex::new(inbox_rx),
            _phantom: PhantomData,
            pipeline: RwLock::new(Pipeline::default()),
        }
    }

    /// Background worker that reads frames from the raw transport and dispatches them.
    async fn read_loop(raw: Arc<R>, shared: Arc<SharedState<S>>) {
        loop {
            let raw_bytes = match raw.recv_frame().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("[read_loop] Transport closed: {:?}", e);
                    break;
                }
            };

            let msg: Message<S::Req, S::Res, S::Ev> = match S::C::decode(raw_bytes) {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("[read_loop] Codec error: {:?}", e);
                    continue;
                }
            };

            match &msg.kind {
                MessageKind::Response(_) => {
                    let mut pending = shared.pending.lock().await;
                    // If a specific caller is waiting for this ID, send it via oneshot
                    if let Some(tx) = pending.remove(&msg.id) {
                        if let MessageKind::Response(payload) = msg.kind {
                            let _ = tx.send(payload);
                        }
                    } else {
                        // Otherwise, push it to the general inbox
                        let _ = shared.inbox_tx.send(msg).await;
                    }
                }
                _ => {
                    // Other always go to the general inbox
                    let _ = shared.inbox_tx.send(msg).await;
                }
            }
        }

        // Clean up pending requests on transport failure
        shared.pending.lock().await.clear();
    }

    /// Registers a new middleware into the transport's processing pipeline.
    ///
    /// Middlewares are executed in the order they are added. They can be used for
    /// logging, metrics collection, authentication, or modifying messages
    /// before they reach the application logic.
    ///
    /// # Guard
    /// This method acquires a write lock on the internal pipeline. It should
    /// ideally be called during the initialization phase of the transport.
    pub async fn add_middleware<M: Middleware<R, S>>(&mut self, middleware: M) {
        let mut p = self.pipeline.write().await;
        p.add_middleware(middleware);
    }

    /// Sends a low-level message envelope directly through the transport.
    ///
    /// This is a "fire-and-forget" operation. It does not track responses or
    /// generate IDs; it simply encodes the provided [`Message`] and pushes
    /// the frame to the underlying [`RawTransport`].
    ///
    /// Use this for emitting events or manual protocol handling.
    pub async fn send(&self, msg: Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        let encoded = S::C::encode(&msg)?;
        self.raw_transport.send_frame(&encoded).await
    }

    /// Performs a synchronized Request-Response operation.
    ///
    /// This high-level method automates the entire RPC lifecycle:
    /// 1. **ID Generation**: Atomically increments the internal counter to ensure a unique `id`.
    /// 2. **Registration**: Creates a `oneshot` channel and registers it in the shared
    ///    `pending` map so the background worker can route the response back.
    /// 3. **Transmission**: Encodes and sends the request frame.
    /// 4. **Awaiting**: Suspend execution until the matching response is received
    ///    or the connection is dropped.
    ///
    /// # Errors
    /// * Returns [`TransportError::SerializeError`] if serialization fails.
    /// * Returns [`TransportError::ConnectionClosed`] if the background worker
    ///   is unable to deliver the response (e.g., socket disconnected).
    pub async fn request(
        &self,
        request: S::Req,
        timeout_secs: u64,
    ) -> Result<S::Res, TransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.shared.pending.lock().await;
            pending.insert(id, tx);
        }

        let msg: Message<S::Req, S::Res, S::Ev> = Message::request(id, request);
        let encoded = S::C::encode(&msg)?;

        if let Err(e) = self.raw_transport.send_frame(&encoded).await {
            let mut pending = self.shared.pending.lock().await;
            pending.remove(&id);
            return Err(e);
        }

        match timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(result) => result.map_err(|_| TransportError::ConnectionClosed),
            Err(_) => {
                let mut pending = self.shared.pending.lock().await;
                pending.remove(&id);

                Err(TransportError::Timeout {
                    seconds: timeout_secs,
                })
            }
        }
    }

    /// Receives the next incoming message directly from the inbox, bypassing the middleware pipeline.
    ///
    /// This is a low-level method that returns a [`MessageContext`] without triggering
    /// any registered middlewares. Use this when you need to handle raw messages
    /// (e.g., control frames, heartbeats) or for debugging purposes where
    /// middleware interference is undesirable.
    ///
    /// # Errors
    /// Returns [`TransportError::ConnectionClosed`] if the underlying connection
    /// or the background worker has been terminated.
    ///
    /// # Locking
    /// This method locks the internal `inbox_rx` mutex. Multiple tasks can call
    /// `next`, but they will be processed sequentially.
    pub async fn next(&self) -> Result<MessageContext<'_, R, S>, TransportError> {
        let mut rx = self.inbox_rx.lock().await;
        let message = rx.recv().await.ok_or(TransportError::ConnectionClosed)?;
        Ok(MessageContext::new(message, self))
    }

    /// Receives the next incoming message and processes it through the middleware pipeline.
    ///
    /// This is the primary method for receiving messages in a production environment.
    /// It first retrieves a message using [`Self::next`] and then asynchronously
    /// executes all registered middlewares via the [`Pipeline`].
    ///
    /// # Pipeline Execution
    /// If any middleware in the pipeline returns an error or chooses to drop the
    /// message (short-circuiting), this method will return that error, and the
    /// message will not be returned to the caller.
    ///
    /// # Returns
    /// Returns a [`MessageContext`] that has successfully passed through all
    /// middleware layers.
    pub async fn recv(&self) -> Result<MessageContext<'_, R, S>, TransportError> {
        let message = self.next().await?;
        {
            let p = self.pipeline.read().await;
            p.execute(&message).await?;
        }

        Ok(message)
    }

    /// Starts a continuous message processing loop using the provided handler.
    ///
    /// This is the primary entry point for building a server or a long-running
    /// message processor with the Knot transport. It manages the full lifecycle
    /// of every incoming message, from the raw wire to your business logic.
    ///
    /// ### Processing Lifecycle
    ///
    /// 1. **Receive**: Awaits the next [`MessageContext`] from the internal inbox.
    /// 2. **Middleware**: Executes the registered [`Middleware`] pipeline.
    ///    Middlewares can inspect, modify, or block the message.
    /// 3. **Dispatch**: If the pipeline completes successfully, the message is
    ///    passed to the terminal `handler`.
    /// 4. **Error Handling**: Non-fatal errors in the pipeline or handler are
    ///    logged, and the loop proceeds to the next message.
    ///
    /// ### Technical Note: `AsyncHandler`
    ///
    /// The handler uses the [`AsyncHandler`] trait with **Higher-Rank Trait Bounds** /// (`for<'a>`). This ensures that the handler can safely borrow data from the
    /// message context during its execution without requiring expensive heap
    /// allocations (like `Box<dyn Future>`) or complex lifetime annotations
    /// in the user's code.
    ///
    /// ### Example
    ///
    /// ```rust,ignore
    /// transport.serve_with(async |ctx| {
    ///     match ctx.kind() {
    ///         MessageKind::Request(req) => {
    ///             println!("Received request: {:?}", req);
    ///             ctx.reply(MyResponse::Ok).await?;
    ///         }
    ///         _ => {}
    ///     }
    ///     Ok(())
    /// }).await;
    /// ```
    ///
    /// # Panics
    ///
    /// This method does not typically panic, but it will propagate panics that
    /// occur within the `handler` or `middleware` unless they are caught
    /// by a specific middleware layer.
    pub async fn serve_with<F>(&self, handler: F)
    where
        F: for<'a> AsyncHandler<'a, R, S>,
    {
        while let Ok(ctx) = self.next().await {
            if let Err(e) = self.pipeline.read().await.execute(&ctx).await {
                eprintln!("Middleware blocked message: {:?}", e);
                continue;
            }

            if let Err(e) = handler.call(ctx).await {
                eprintln!("Handler error: {:?}", e);
            }
        }
    }
}
