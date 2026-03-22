use crate::messages::{Message, MessageKind};
use crate::codec::MessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{mpsc, oneshot, Mutex};
use std::marker::PhantomData;

pub mod ipc;


#[async_trait]
pub trait RawTransport: Send + Sync + Sized {
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError>;
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError>;
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

#[derive(Debug)]
pub struct SharedState<TRequest, TResponse> {
    pub pending: Mutex<HashMap<u32, PendingResponse<TResponse>>>,
    pub inbox_tx: mpsc::Sender<Message<TRequest, TResponse>>,
}

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
    pub fn new(raw: R) -> Self {
        let (inbox_tx, inbox_rx) = mpsc::channel(32);
        let raw_transport = Arc::new(raw);

        let shared = Arc::new(SharedState {
            pending: Mutex::new(HashMap::new()),
            inbox_tx,
        });

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

    async fn read_loop(raw: Arc<R>, shared: Arc<SharedState<TRequest, TResponse>>) {
        loop {
            let raw_bytes = match raw.recv_frame().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("[read_loop] Raw transport closed/error: {:?}", e);
                    break;
                }
            };

            let msg: Message<TRequest, TResponse> = match TCodec::decode(raw_bytes) {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("[read_loop] Codec decode error: {:?}", e);
                    continue;
                }
            };

            match &msg.kind {
                MessageKind::Response(_) => {
                    let mut pending = shared.pending.lock().await;
                    eprintln!("[read_loop] pending count={}", pending.len());

                    if let Some(tx) = pending.remove(&msg.id) {
                        if let MessageKind::Response(payload) = msg.kind {
                            let _ = tx.send(payload);
                        }
                    } else {
                        let _ = shared.inbox_tx.send(msg).await;
                    }
                }
                MessageKind::Request(_) => {
                    let _ = shared.inbox_tx.send(msg).await;
                }
            }
        }
        
        shared.pending.lock().await.clear();
    }

    pub async fn send(&self, msg: Message<TRequest, TResponse>) -> Result<(), TransportError> {
        let encoded = TCodec::encode(&msg)?;
        self.raw_transport.send_frame(&encoded).await
    }

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

    pub async fn recv(&self) -> Result<Message<TRequest, TResponse>, TransportError> {
        let mut rx = self.inbox_rx.lock().await;
        rx.recv().await.ok_or(TransportError::ConnectionClosed)
    }
}

#[async_trait]
pub trait Server {
    type Address: Send;
    type Transport: RawTransport;

    async fn bind(addr: Self::Address) -> Result<Self, TransportError> 
    where 
        Self: Sized;

    async fn shutdown(&mut self);
    async fn accept(&self) -> Result<Self::Transport, TransportError>;
}