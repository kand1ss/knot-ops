//! # Knot Transport
//!
//! This module provides the infrastructure for typed, asynchronous messaging.
//! It handles frame-based I/O, serialization, and request-response synchronization.

use crate::codec::MessageCodec;
use crate::messages::{Message, MessageContext, MessageKind, MetadataMap};
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

type MessageSender<S> = oneshot::Sender<
    Message<<S as TransportSpec>::Req, <S as TransportSpec>::Res, <S as TransportSpec>::Ev>,
>;
type PendingMap<S> = HashMap<u32, MessageSender<S>>;

/// Maximum allowed size for a single message (10 MB).
/// Acts as a safeguard against memory exhaustion from malformed packets.
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Internal state shared between the public API and the background read loop.
#[derive(Debug)]
pub struct SharedState<S: TransportSpec> {
    /// Tracks requests waiting for a response.
    pub pending: Mutex<PendingMap<S>>,
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
                        if let MessageKind::Response(_) = msg.kind {
                            let _ = tx.send(msg);
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

    /// Sends a low-level message envelope through the outbound middleware pipeline.
    ///
    /// This is a "fire-and-forget" operation that performs the following steps:
    /// 1. **Middleware Processing**: Passes the [`Message`] through the outbound pipeline
    ///    via `execute_send`. Middleware may modify metadata or block the message.
    /// 2. **Encoding**: Serializes the (potentially modified) message using the codec.
    /// 3. **Transmission**: Pushes the resulting frame to the underlying [`RawTransport`].
    ///
    /// Use this for emitting events, manual protocol handling, or sending raw responses.
    ///
    /// # Errors
    /// * Returns [`TransportError::MiddlewareBlocked`] if a middleware halts the execution.
    /// * Returns [`TransportError::SerializeError`] if serialization fails.
    pub async fn send(
        &self,
        mut msg: Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        self.pipeline.read().await.execute_send(&mut msg).await?;
        let encoded = S::C::encode(&msg)?;
        self.raw_transport.send_frame(&encoded).await
    }

    /// Performs a synchronized Request-Response operation with full context and middleware interception.
    ///
    /// This method automates the entire RPC lifecycle, ensuring that both the outgoing request
    /// and the incoming response pass through the configured [`Middleware`] layers.
    /// It returns a [`MessageContext`], allowing access to response metadata and the transport handle.
    ///
    /// ### Workflow:
    /// 1. **ID Generation**: Assigns a unique sequence ID to the request for correlation.
    /// 2. **Outbound Pipeline**: Wraps the request and optional `metadata` into a [`Message`],
    ///    then executes `on_send` middleware hooks via [`Self::send`].
    /// 3. **Response Tracking**: Registers a `oneshot` observer in the internal pending map.
    /// 4. **Await & Timeout**: Suspends execution until the background worker receives a matching
    ///    frame or the `timeout_secs` duration expires.
    /// 5. **Inbound Pipeline**: The received response is processed by `on_recv` middleware hooks.
    /// 6. **Context Wrapping**: Returns a [`MessageContext`] containing the final processed message.
    ///
    /// # Arguments
    /// * `request` - The request payload (defined by the [`TransportSpec`]).
    /// * `timeout_secs` - Seconds to wait before returning a [`TransportError::Timeout`].
    /// * `metadata` - Optional key-value pairs to attach to the outgoing request.
    ///
    /// # Errors
    /// * [`TransportError::Timeout`]: No response received within the allotted time.
    /// * [`TransportError::MiddlewareBlocked`]: A middleware layer rejected the message.
    /// * [`TransportError::ConnectionClosed`]: The underlying transport or worker task failed.
    pub async fn request_full(
        &self,
        request: S::Req,
        timeout_secs: u64,
        metadata: Option<MetadataMap>,
    ) -> Result<MessageContext<'_, R, S>, TransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.shared.pending.lock().await;
            pending.insert(id, tx);
        }

        let msg = Message::request(id, request).maybe_with_metadata(metadata);
        if let Err(e) = self.send(msg).await {
            eprintln!("error");
            let mut pending = self.shared.pending.lock().await;
            pending.remove(&id);
            return Err(e);
        }

        match timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(result) => {
                let msg = result.map_err(|_| TransportError::ConnectionClosed)?;
                self.pipeline.read().await.execute_recv(&msg).await?;
                Ok(MessageContext::new(msg, self))
            }
            Err(_) => {
                let mut pending = self.shared.pending.lock().await;
                pending.remove(&id);

                Err(TransportError::Timeout {
                    seconds: timeout_secs,
                })
            }
        }
    }

    /// A simplified version of [`Self::request_full`] that returns only the response payload.
    ///
    /// Use this method when you do not need to inspect response metadata or use the
    /// [`MessageContext`] for further replies. It internally calls `request_full` and
    /// extracts the inner response data.
    ///
    /// # Errors
    /// In addition to the errors defined in [`Self::request_full`], this returns
    /// [`TransportError::DeserializeError`] if the received message is not a valid response kind.
    pub async fn request(
        &self,
        request: S::Req,
        timeout_secs: u64,
        metadata: Option<MetadataMap>,
    ) -> Result<S::Res, TransportError> {
        let ctx = self.request_full(request, timeout_secs, metadata).await?;
        let (msg, _) = ctx.into_parts();

        match msg.into_response() {
            Some((_, res)) => Ok(res),
            None => Err(TransportError::DeserializeError {
                reason: "expected another message kind".to_string(),
            }),
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
        let ctx = self.next().await?;
        {
            let p = self.pipeline.read().await;
            p.execute_recv(ctx.get()).await?;
        }

        Ok(ctx)
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
    /// transport.serve_with(async |mut ctx| {
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
            if let Err(e) = self.pipeline.read().await.execute_recv(ctx.get()).await {
                eprintln!("Middleware blocked message: {:?}", e);
                continue;
            }

            if let Err(e) = handler.call(ctx).await {
                eprintln!("Handler error: {:?}", e);
            }
        }
    }
}
