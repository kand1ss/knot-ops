use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tonic::transport::server::Connected;

use interprocess::local_socket::tokio::prelude::LocalSocketStream;

/// Metadata associated with an IPC connection.
///
/// Since IPC transport (Unix Domain Sockets / Named Pipes) lacks the concept of
/// remote IP addresses or ports typical of TCP/IP, this struct acts as a
/// placeholder for connection information requested by `tonic`.
#[derive(Clone, Debug)]
pub struct IpcConnectInfo;

/// A wrapper around a local socket stream that bridges it to the `tonic` transport layer.
///
/// This struct implements the `Connected` trait required by `tonic::transport::server::Connected`,
/// allowing the gRPC server to treat IPC streams as first-class transport citizens.
/// It acts as a transparent proxy for IO operations to the underlying
/// `interprocess::local_socket::tokio::Stream`.
#[derive(Debug)]
pub struct IpcServerStream {
    /// The underlying IPC stream managed by `interprocess`.
    inner: LocalSocketStream,
}

impl IpcServerStream {
    /// Wraps an existing `LocalSocketStream` into an `IpcServerStream`.
    pub fn new(inner: LocalSocketStream) -> Self {
        Self { inner }
    }
}

impl Connected for IpcServerStream {
    type ConnectInfo = IpcConnectInfo;

    /// Returns the connection metadata.
    ///
    /// For IPC, this returns an empty `IpcConnectInfo` instance, as there is no
    /// network-level address information available.
    fn connect_info(&self) -> Self::ConnectInfo {
        IpcConnectInfo
    }
}

impl AsyncRead for IpcServerStream {
    /// Polls the inner stream for data read operations.
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for IpcServerStream {
    /// Polls the inner stream for data write operations.
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    /// Flushes the inner stream.
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    /// Shuts down the inner stream.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
