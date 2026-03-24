//! # Knot Transport
//!
//! This module provides the infrastructure for typed, asynchronous messaging.
//! It handles frame-based I/O, serialization, and request-response synchronization.

use crate::codec::MessageCodec;
use crate::messages::{Message, MessageContext, MessageKind};
use knot_core::errors::TransportError;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::{sync::{Mutex, mpsc, oneshot}, time::{timeout, Duration}};

pub mod ipc;
mod traits;

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
    R: RawTransport + 'static,
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
}

impl<R, S> MessageTransport<R, S>
where
    R: RawTransport + 'static,
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
    pub async fn request(&self, request: S::Req, timeout_secs: u64) -> Result<S::Res, TransportError> {
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
        Ok(result) => {
            result.map_err(|_| TransportError::ConnectionClosed)
        }
        Err(_) => {
            let mut pending = self.shared.pending.lock().await;
            pending.remove(&id);
            
            Err(TransportError::Timeout { seconds: timeout_secs }) 
        }
    }
    }

    /// Receives the next incoming message wrapped in a [`MessageContext`].
    ///
    /// This method awaits the next message from the internal inbox. The returned
    /// context provides easy access to the message data and a convenient way
    /// to send replies or emit events back through this transport.
    ///
    /// # Errors
    /// Returns [`TransportError::ConnectionClosed`] if the background worker
    /// or the underlying connection has been terminated.
    ///
    /// # Locking
    /// This method locks the internal `inbox_rx` mutex. While multiple tasks
    /// can call `recv`, they will be processed sequentially.
    pub async fn recv(&self) -> Result<MessageContext<'_, R, S>, TransportError> {
        let mut rx = self.inbox_rx.lock().await;
        let message = rx.recv().await.ok_or(TransportError::ConnectionClosed)?;
        Ok(MessageContext::new(message, self))
    }
}
