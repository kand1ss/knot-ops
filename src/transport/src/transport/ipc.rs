use std::path::PathBuf;

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
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, WriteHalf, ReadHalf}, 
    sync::Mutex
};

use knot_core::errors::TransportError;
use crate::transport::RawTransport;


const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;


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
    

use std::sync::Arc;

#[derive(Debug)]
pub struct IpcTransport {
    writer: Arc<Mutex<WriteHalf<LocalSocketStream>>>,
    reader: Arc<Mutex<ReadHalf<LocalSocketStream>>>,
}

impl IpcTransport {
    pub fn from_socket(stream: LocalSocketStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            writer: Arc::new(Mutex::new(writer)),
            reader: Arc::new(Mutex::new(reader)),
        }
    }

    pub async fn connect(socket_path: PathBuf) -> Result<Self, TransportError> {
        let name = resolve_socket_name(&socket_path)?;
        let stream = LocalSocketStream::connect(name)
            .await
            .map_err(|e| TransportError::ConnectionFailed {
                path: socket_path.clone(),
                source: e,
            })?;

        Ok(Self::from_socket(stream))
    }

    async fn read_frame_internal(reader: &mut ReadHalf<LocalSocketStream>) -> Result<Vec<u8>, TransportError> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => TransportError::ConnectionClosed,
            _ => TransportError::Io { source: e },
        })?;
        
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_MESSAGE_SIZE {
            return Err(TransportError::MessageTooLarge { size: len });
        }

        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await.map_err(|e| TransportError::Io { source: e })?;
        Ok(buf)
    }
}

#[async_trait]
impl RawTransport for IpcTransport {
    async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
        let mut writer = self.writer.lock().await;
        let len = frame.len() as u32;

        writer.write_all(&len.to_be_bytes()).await
            .map_err(|e| TransportError::Io { source: e })?;

        writer.write_all(frame).await
            .map_err(|e| TransportError::Io { source: e })?;

        Ok(())
    }

    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError> {
        let mut reader = self.reader.lock().await;
        Self::read_frame_internal(&mut reader).await
    }
}


use crate::transport::Server;

pub struct IpcServer {
    listener: LocalSocketListener,
    socket_path: PathBuf,
    is_closed: bool,
}

impl IpcServer {
    pub fn path(&self) -> &PathBuf {
        &self.socket_path
    }
}

#[async_trait]
impl Server for IpcServer {
    type Address = PathBuf;
    type Transport = IpcTransport;

    async fn bind(socket_path: PathBuf) -> Result<Self, TransportError> {
        let name = resolve_socket_name(&socket_path)?;
        
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .map_err(|e| TransportError::ConnectionFailed {
                path: socket_path.clone(),
                source: e,
            })?;

        Ok(Self { 
            listener, 
            socket_path, 
            is_closed: false 
        })
    }

    async fn shutdown(&mut self) {
        if self.is_closed {
            return;
        }

        #[cfg(unix)]
        {
            if self.socket_path.exists() {
                let _ = tokio::fs::remove_file(&self.socket_path).await;
            }
        }
        self.is_closed = true;
    }

    async fn accept(&self) -> Result<Self::Transport, TransportError> {
        let stream = self.listener
            .accept()
            .await
            .map_err(|e| TransportError::Io { source: e })?;
        
        Ok(IpcTransport::from_socket(stream))
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if !self.is_closed {
            #[cfg(unix)]
            {
                // В синхронном Drop используем std::fs
                let _ = std::fs::remove_file(&self.socket_path);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

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