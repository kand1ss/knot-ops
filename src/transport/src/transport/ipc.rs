use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
use std::marker::PhantomData;

use async_trait::async_trait;
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::{
    GenericNamespaced,
    GenericFilePath,
    Name,
    ListenerOptions,
    ToFsName,
    ToNsName
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, WriteHalf, ReadHalf}, 
    sync::{Mutex, oneshot}
};

use knot_core::errors::TransportError;
use crate::{
    messages::{Message, MessageKind},
    codec::MessageCodec,
    transport::Transport,
};


fn resolve_socket_name(path: &PathBuf) -> Result<Name<'static>, TransportError> {
        if cfg!(windows) {
            path.file_name()
                .and_then(|n| n.to_str())
                .ok_or(TransportError::InvalidSocketPath {
                    path: path.clone()
                })?
                .to_ns_name::<GenericNamespaced>()
                .map(|name| name.into_owned())
                .map_err(|_e| TransportError::InvalidSocketPath {
                    path: path.clone()
                })
        } else {
            path.clone()
                .to_fs_name::<GenericFilePath>()
                .map_err(|_e| TransportError::InvalidSocketPath {
                    path: path.clone()
                })
        }
    }
    

use std::collections::HashMap;
use tokio::sync::mpsc;

struct Shared<TRequest, TResponse> {
    pending: Mutex<HashMap<u32, oneshot::Sender<TResponse>>>,
    inbox: mpsc::Sender<Message<TRequest, TResponse>>
}

pub struct IpcTransport<TRequest, TResponse, TProtocol>
where
    TRequest:  Serialize + DeserializeOwned + Send,
    TResponse: Serialize + DeserializeOwned + Send,
    TProtocol: MessageCodec<Raw = Vec<u8>>,
{
    writer: Arc<Mutex<WriteHalf<LocalSocketStream>>>,
    next_id: AtomicU32,
    shared: Arc<Shared<TRequest, TResponse>>,
    inbox_rx: Mutex<mpsc::Receiver<Message<TRequest, TResponse>>>,
    _phantom: PhantomData<(TRequest, TResponse, TProtocol)>,
}

impl<TRequest, TResponse, TProtocol> IpcTransport<TRequest, TResponse, TProtocol>
where
    TRequest:  Serialize + DeserializeOwned + Send + 'static,
    TResponse: Serialize + DeserializeOwned + Send + 'static,
    TProtocol: MessageCodec<Raw = Vec<u8>> + Send + 'static,
{
    async fn from_socket(stream: LocalSocketStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        let (inbox_tx, inbox_rx) = mpsc::channel(32);

        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            inbox: inbox_tx
        });
        
        tokio::spawn(Self::read_loop(
            reader, 
            Arc::clone(&shared)
        ));

        Self {
            writer: Arc::new(Mutex::new(writer)),
            inbox_rx: Mutex::new(inbox_rx),
            shared,
            next_id: AtomicU32::new(0),
            _phantom: PhantomData,
        }
    }

    async fn read_loop(mut reader: ReadHalf<LocalSocketStream>, shared: Arc<Shared<TRequest, TResponse>>) {
        loop {
            let raw = match Self::read_frame(&mut reader).await {
                Ok(raw) => raw,
                Err(e)  => {
                    eprintln!("[read_loop] read_frame error: {:?}", e);
                    break
                },
            };

            let msg: Message<TRequest, TResponse> = match TProtocol::decode(raw) {
                Ok(msg) => msg,
                Err(e)  => {
                    eprintln!("[read_loop] deserialize error: {:?}", e);
                    break
                },
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
                        let _ = shared.inbox.send(msg).await;
                    }
                }
                MessageKind::Request(_) => {
                    let _ = shared.inbox.send(msg).await;
                }
            }
        }

        shared.pending.lock().await.clear();
    }

    async fn write_frame(
        writer: &mut WriteHalf<LocalSocketStream>,
        data:   &[u8],
    ) -> Result<(), TransportError> {
        let len = data.len() as u32;

        writer
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        writer
            .write_all(data)
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        Ok(())
    }

    async fn read_frame(
        reader: &mut ReadHalf<LocalSocketStream>,
    ) -> Result<Vec<u8>, TransportError> {
        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => TransportError::ConnectionClosed,
                _ => TransportError::Io { source: e },
            })?;

        const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
        
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge { size: len });
        }

        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        Ok(buf)
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn send_message(
        &self,
        msg: &Message<TRequest, TResponse>,
    ) -> Result<(), TransportError> {
        let mut writer = self.writer.lock().await;
        let raw = TProtocol::encode(msg)?;
        Self::write_frame(&mut writer, &raw).await
    }

    async fn recv_message(
        &self,
    ) -> Result<Message<TRequest, TResponse>, TransportError> {
        self.inbox_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(TransportError::ConnectionClosed)
    }
}

#[async_trait]
impl<TRequest, TResponse, TProtocol> Transport<TRequest, TResponse, TProtocol>
    for IpcTransport<TRequest, TResponse, TProtocol>
where
    TRequest:  Serialize + DeserializeOwned + Send + Sync + 'static,
    TResponse: Serialize + DeserializeOwned + Send + Sync + 'static,
    TProtocol: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    type Address = PathBuf;

    async fn connect(socket_path: PathBuf) -> Result<Self, TransportError> {
        let name = resolve_socket_name(&socket_path)?;
        let stream = LocalSocketStream::connect(name)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    TransportError::ConnectionRefused
                }
                std::io::ErrorKind::NotFound => {
                    TransportError::InvalidSocketPath {
                        path: socket_path.clone(),
                    }
                }
                _ => TransportError::ConnectionFailed {
                    path:   socket_path.clone(),
                    source: e,
                },
            })?;

        Ok(Self::from_socket(stream).await)
    }

    async fn serve(
        &self,
    ) -> Result<Message<TRequest, TResponse>, TransportError> {
        self.recv_message().await
    }

    async fn send(
        &self,
        message: Message<TRequest, TResponse>,
    ) -> Result<(), TransportError> {
        self.send_message(&message).await
    }

    async fn request(
        &self,
        request: TRequest,
    ) -> Result<TResponse, TransportError> {
        let id  = self.next_id();
        let msg = Message::request(id, request);

        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().await.insert(id, tx);

        self.send_message(&msg).await?;
        rx.await.map_err(|_| TransportError::ConnectionClosed)
    }
}


use crate::transport::Server;


pub struct IpcServer {
    listener: LocalSocketListener,
    socket_path: PathBuf,
    is_closed: bool
}
impl IpcServer {
    pub fn path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub async fn accept_specific<TRequest, TResponse, TProtocol>(&self) 
        -> Result<IpcTransport<TRequest, TResponse, TProtocol>, TransportError>
    where
        TRequest: Serialize + DeserializeOwned + Send + Sync + 'static,
        TResponse: Serialize + DeserializeOwned + Send + Sync + 'static,
        TProtocol: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
    {
        let stream = self.listener
            .accept()
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        Ok(IpcTransport::from_socket(stream).await)
    }
}

#[async_trait]
impl Server for IpcServer {
    type Address = PathBuf;

    async fn bind(socket_path: PathBuf) -> Result<Self, TransportError> {
        let name = resolve_socket_name(&socket_path)?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .map_err(|e| TransportError::ConnectionFailed {
                path:   socket_path.clone(),
                source: e,
            })?;

        Ok(Self { listener, socket_path, is_closed: false })
    }

    async fn shutdown(&mut self) {
        if self.is_closed {
            return;
        }

        #[cfg(unix)]
        {
            if self.socket_path.exists() {
                let res = tokio::fs::remove_file(&self.socket_path).await;
            }
        }
        self.is_closed = true;
    }

    async fn accept<TRequest, TResponse, TProtocol>(&self) 
        -> Result<Box<dyn Transport<TRequest, TResponse, TProtocol, Address = Self::Address>>, TransportError>
    where
        TRequest: Serialize + DeserializeOwned + Send + Sync + 'static,
        TResponse: Serialize + DeserializeOwned + Send + Sync + 'static,
        TProtocol: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
    {
        let transport = self.accept_specific().await?;
        Ok(Box::new(transport))
    }
}
impl Drop for IpcServer {
    fn drop(&mut self) {
        if !self.is_closed {
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&self.socket_path);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::daemon::{DaemonRequest, DaemonResponse};
    use crate::codec::JsonCodec;

    #[tokio::test]
    async fn test_frame_roundtrip() {
        let (mut client, mut server) = tokio::io::duplex(1024);

        let data = b"hello knot";

        let len = data.len() as u32;
        client.write_all(&len.to_be_bytes()).await.unwrap();
        client.write_all(data).await.unwrap();

        let mut len_buf = [0u8; 4];
        server.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        server.read_exact(&mut buf).await.unwrap();

        assert_eq!(&buf, data);
    }

    #[test]
    fn test_json_protocol_serialize_deserialize() {
        let msg: Message<DaemonRequest, DaemonResponse> = Message::request(
            42,
            DaemonRequest::Down,
        );

        let raw = JsonCodec::encode(&msg).unwrap();
        let decoded: Message<DaemonRequest, DaemonResponse> =
            JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(decoded.kind, MessageKind::Request(_)));
    }

    #[test]
    fn test_json_deserialize_response() {
        let msg: Message<DaemonRequest, DaemonResponse> = Message::response(
            42,
            DaemonResponse::Status { services: vec![] },
        );

        let raw     = JsonCodec::encode(&msg).unwrap();
        let decoded: Message<DaemonRequest, DaemonResponse> =
            JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(
            decoded.kind,
            MessageKind::Response(DaemonResponse::Status { .. })
        ));
    }

    #[test]
    fn test_json_deserialize_invalid_data() {
        let invalid = b"not valid json at all {{{".to_vec();

        let result: Result<Message<DaemonRequest, DaemonResponse>, _> =
            JsonCodec::decode(invalid);

        assert!(matches!(
            result,
            Err(TransportError::DeserializeError { .. })
        ));
    }

    #[test]
    fn test_message_request_response_roundtrip() {
        let request: Message<DaemonRequest, DaemonResponse> = Message::request(
            1,
            DaemonRequest::Down,
        );

        let raw    = JsonCodec::encode(&request).unwrap();
        let decoded: Message<DaemonRequest, DaemonResponse> =
            JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 1);
        assert!(decoded.timestamp > 0);
    }

    #[test]
    fn test_resolve_socket_name_valid_path() {
        let path = PathBuf::from("/tmp/knot-test.sock");
        let result = resolve_socket_name(&path);
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_socket_name_uses_full_path() {
        let path   = PathBuf::from("/tmp/knot-test.sock");
        let name   = resolve_socket_name(&path).unwrap();
        let name_str = name.to_string();
        assert!(name_str.contains("knot-test.sock"));
    }

    #[test]
    fn test_resolve_socket_name_empty_path_fails() {
        let path = PathBuf::from("");
        let result = resolve_socket_name(&path);
        assert!(
            result.is_err(),
            "empty path should return error"
        );
    }
}