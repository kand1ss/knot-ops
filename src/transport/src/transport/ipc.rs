//! # IPC Transport Implementation
//!
//! This module provides a concrete implementation of the `RawTransport` and `Server`
//! traits using local sockets (Unix Domain Sockets on Unix or Named Pipes on Windows).
//!
//! It uses a length-prefixed framing protocol to ensure message integrity
//! across the stream.

use async_trait::async_trait;
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, ToFsName, ToNsName,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    sync::Mutex,
};
use tracing::{debug, error, info, instrument, warn};

use crate::transport::{MAX_MESSAGE_SIZE, RawTransport};
use knot_core::errors::TransportError;

/// Resolves a filesystem path into a cross-platform local socket name.
///
/// On Windows, it extracts the file name for use in the object namespace.
/// On Unix, it uses the direct filesystem path for the socket file.
fn resolve_socket_name(path: &Path) -> Result<Name<'static>, TransportError> {
    if path.as_os_str().is_empty() {
        return Err(TransportError::InvalidSocketPath {
            path: path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    if path.as_os_str().len() > 100 {
        return Err(TransportError::InvalidSocketPath {
            path: path.to_path_buf(),
        });
    }

    if cfg!(windows) {
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or(TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })?
            .to_ns_name::<GenericNamespaced>()
            .map(|name| name.into_owned())
            .map_err(|_e| TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })
    } else {
        path.to_path_buf()
            .to_fs_name::<GenericFilePath>()
            .map_err(|_e| TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })
    }
}

/// A transport implementation that uses local sockets for Inter-Process Communication.
///
/// Internally, it splits the socket into a reader and a writer, both protected
/// by Mutexes to allow concurrent `send` and `recv` operations.
#[derive(Debug)]
pub struct IpcTransport {
    writer: Arc<Mutex<WriteHalf<LocalSocketStream>>>,
    reader: Arc<Mutex<ReadHalf<LocalSocketStream>>>,
}

impl IpcTransport {
    /// Creates an `IpcTransport` from an existing `LocalSocketStream`.
    pub fn from_socket(stream: LocalSocketStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            writer: Arc::new(Mutex::new(writer)),
            reader: Arc::new(Mutex::new(reader)),
        }
    }

    /// Establishes a new connection to a local socket at the specified path.
    #[instrument(skip(socket_path), fields(path = %socket_path.display()), err)]
    pub async fn connect(socket_path: PathBuf) -> Result<Self, TransportError> {
        let name = resolve_socket_name(&socket_path)?;

        debug!("Connecting to local socket...");
        let stream = LocalSocketStream::connect(name).await.map_err(|e| {
            TransportError::ConnectionFailed {
                path: socket_path.clone(),
                source: e,
            }
        })?;

        info!("Connection established");
        Ok(Self::from_socket(stream))
    }

    /// Internal logic for reading a length-prefixed frame from the stream.
    ///
    /// The protocol expects a 4-byte Big-Endian length header followed by the payload.
    async fn read_frame_internal(
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

        let len = u32::from_be_bytes(len_buf) as usize;

        // Safety check to prevent allocating huge buffers from corrupted data
        if len > MAX_MESSAGE_SIZE {
            warn!("Too large message, frame skipped...");
            return Err(TransportError::MessageTooLarge { size: len });
        }

        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|e| TransportError::Io { source: e })?;
        Ok(buf)
    }
}

#[async_trait]
impl RawTransport for IpcTransport {
    /// Encodes and sends a frame using a 4-byte length prefix (Big-Endian).
    async fn send_frame_internal<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
        debug!("Locking writer...");
        let mut writer = self.writer.lock().await;
        debug!("Writer locked");
        let len = frame.len() as u32;

        // Write header
        writer
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        // Write payload
        writer
            .write_all(frame)
            .await
            .map_err(|e| TransportError::Io { source: e })?;

        debug!("Frame sent successfully, unlocking writer...");
        Ok(())
    }

    /// Locks the reader and waits for the next incoming frame.
    async fn recv_frame_internal(&self) -> Result<Vec<u8>, TransportError> {
        debug!("Waiting for reader lock...");
        let mut reader = self.reader.lock().await;
        debug!("Reader locked");
        let res = Self::read_frame_internal(&mut reader).await;
        debug!("Unlocking reader lock...");
        res
    }
}

use crate::transport::Server;

/// A server implementation that listens for incoming local socket connections.
pub struct IpcServer {
    listener: LocalSocketListener,
    socket_path: PathBuf,
    is_closed: bool,
}

impl IpcServer {
    /// Returns the filesystem path where the server is listening.
    pub fn path(&self) -> &PathBuf {
        &self.socket_path
    }
}

#[async_trait]
impl Server for IpcServer {
    type Address = PathBuf;
    type Transport = IpcTransport;

    /// Binds a local socket listener to the provided path.
    #[instrument(skip(socket_path), fields(path = %socket_path.display()), err)]
    async fn bind(socket_path: PathBuf) -> Result<Self, TransportError> {
        #[cfg(unix)]
        {
            use tracing::warn;

            debug!("Socket file already exists, attempting to remove...");
            if let Err(e) = tokio::fs::remove_file(&socket_path).await {
                warn!(error = %e, "Failed to remove existing socket file, bind might fail");
            } else {
                debug!("Existing socket file removed successfully");
            }
        }

        let name = resolve_socket_name(&socket_path)?;

        debug!("Creating tokio local socket listener...");
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .map_err(|e| {
                error!(error = %e, "Failed to create listener");
                TransportError::ConnectionFailed {
                    path: socket_path.clone(),
                    source: e,
                }
            })?;

        info!("IpcServer successfully bound to socket");
        Ok(Self {
            listener,
            socket_path,
            is_closed: false,
        })
    }

    /// Performs an orderly shutdown of the server.
    ///
    /// On Unix systems, it removes the socket file from the filesystem.
    #[instrument(skip(self), fields(path = %self.socket_path.display()))]
    async fn shutdown(&mut self) {
        if self.is_closed {
            debug!("Shutdown called, but server is already closed");
            return;
        }

        info!("Shutting down IpcServer...");

        #[cfg(unix)]
        {
            if self.socket_path.exists() {
                debug!("Removing socket file from filesystem...");
                if let Err(e) = tokio::fs::remove_file(&self.socket_path).await {
                    error!(error = %e, "Failed to remove socket file during shutdown");
                } else {
                    debug!("Socket file removed");
                }
            }
        }
        self.is_closed = true;
        info!("IpcServer shutdown complete");
    }

    /// Accepts the next incoming IPC connection.
    #[instrument(skip(self), name = "ipc_server_accept", err)]
    async fn accept(&self) -> Result<Self::Transport, TransportError> {
        info!("Waiting for next incoming connection...");

        let stream = self.listener.accept().await.map_err(|e| {
            error!(error = %e, "Error while accepting connection");
            TransportError::Io { source: e }
        })?;

        info!("Accepted new connection...");

        #[cfg(unix)]
        if let Ok(cred) = stream.peer_creds() {
            info!(peer_pid = ?cred.pid(), "Accepted new connection...");
        } else {
            info!("Accepted new connection (could not retrieve peer credentials)...");
        }

        Ok(IpcTransport::from_socket(stream))
    }
}

/// Ensures the socket file is cleaned up when the server instance is dropped.
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
        let path = PathBuf::from("/tmp/knot-test.sock");
        let name = resolve_socket_name(&path).unwrap();
        let name_debug = format!("{:?}", name);

        assert!(name_debug.contains("knot-test.sock"));
    }

    #[test]
    fn test_resolve_socket_name_empty_path_fails() {
        let path = PathBuf::from("");
        let result = resolve_socket_name(&path);
        assert!(result.is_err(), "empty path should return error");
    }
}
