//! # Knot Transport
//!
//! This module provides the infrastructure for typed, asynchronous messaging.
//! It handles frame-based I/O, serialization, and request-response synchronization.

use crate::codec::MessageCodec;
use crate::messages::{Message, MessageKind};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, mpsc, oneshot};

pub mod ipc;

/// Defines a low-level byte-frame transport.
///
/// Implementors are responsible for ensuring that a single call to `recv_frame`
/// returns exactly one complete message frame.
#[async_trait]
pub trait RawTransport: Send + Sync + Sized {
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError>;
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError>;

    /// Wraps the raw transport into a high-level `MessageTransport`.
    fn to_messaged<Req, Res, C>(self) -> MessageTransport<Self, Req, Res, C>
    where
        Req: Serialize + DeserializeOwned + Send + 'static,
        Res: Serialize + DeserializeOwned + Send + 'static,
        C: MessageCodec<Raw = Vec<u8>> + Send + 'static,
    {
        MessageTransport::new(self)
    }
}

type PendingResponse<TResponse> = oneshot::Sender<TResponse>;

/// Internal state shared between the public API and the background read loop.
#[derive(Debug)]
pub struct SharedState<TRequest, TResponse> {
    /// Tracks requests waiting for a response.
    pub pending: Mutex<HashMap<u32, PendingResponse<TResponse>>>,
    /// Channel to send incoming requests or unhandled responses to the consumer.
    pub inbox_tx: mpsc::Sender<Message<TRequest, TResponse>>,
}

/// A high-level transport that handles typed messages and request-response pairing.
#[derive(Debug)]
pub struct MessageTransport<R, TRequest, TResponse, TCodec>
where
    R: RawTransport + 'static,
{
    raw_transport: Arc<R>,
    next_id: AtomicU32,
    shared: Arc<SharedState<TRequest, TResponse>>,
    inbox_rx: Mutex<mpsc::Receiver<Message<TRequest, TResponse>>>,
    _phantom: PhantomData<TCodec>,
}

impl<R, TRequest, TResponse, TCodec> MessageTransport<R, TRequest, TResponse, TCodec>
where
    R: RawTransport + 'static,
    TRequest: Serialize + DeserializeOwned + Send + 'static,
    TResponse: Serialize + DeserializeOwned + Send + 'static,
    TCodec: MessageCodec<Raw = Vec<u8>> + Send + 'static,
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
    async fn read_loop(raw: Arc<R>, shared: Arc<SharedState<TRequest, TResponse>>) {
        loop {
            let raw_bytes = match raw.recv_frame().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("[read_loop] Transport closed: {:?}", e);
                    break;
                }
            };

            let msg: Message<TRequest, TResponse> = match TCodec::decode(raw_bytes) {
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
                MessageKind::Request(_) => {
                    // Requests always go to the general inbox
                    let _ = shared.inbox_tx.send(msg).await;
                }
            }
        }

        // Clean up pending requests on transport failure
        shared.pending.lock().await.clear();
    }

    /// Sends a one-way message without waiting for a response.
    pub async fn send(&self, msg: Message<TRequest, TResponse>) -> Result<(), TransportError> {
        let encoded = TCodec::encode(&msg)?;
        self.raw_transport.send_frame(&encoded).await
    }

    /// Sends a request and returns a future that resolves to the matching response.
    ///
    /// This method generates a unique ID, registers a listener, and waits for the
    /// background loop to receive the corresponding response.
    pub async fn request(&self, request: TRequest) -> Result<TResponse, TransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.shared.pending.lock().await;
            pending.insert(id, tx);
        }

        let msg: Message<TRequest, TResponse> = Message::request(id, request);
        let encoded = TCodec::encode(&msg)?;

        if let Err(e) = self.raw_transport.send_frame(&encoded).await {
            let mut pending = self.shared.pending.lock().await;
            pending.remove(&id);
            return Err(e);
        }

        rx.await.map_err(|_| TransportError::ConnectionClosed)
    }

    /// Receives the next available message from the inbox (requests or unhandled responses).
    pub async fn recv(&self) -> Result<Message<TRequest, TResponse>, TransportError> {
        let mut rx = self.inbox_rx.lock().await;
        rx.recv().await.ok_or(TransportError::ConnectionClosed)
    }
}

/// Interface for a network-based server that can accept new connections.
#[async_trait]
pub trait Server {
    type Address: Send;
    type Transport: RawTransport;

    /// Binds to the specified address and starts the listener.
    async fn bind(addr: Self::Address) -> Result<Self, TransportError>
    where
        Self: Sized;

    /// Stops the server listener.
    async fn shutdown(&mut self);

    /// Accepts the next incoming connection.
    async fn accept(&self) -> Result<Self::Transport, TransportError>;
}
