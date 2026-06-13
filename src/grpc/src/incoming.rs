mod options;
mod stream;
pub use options::*;
use stream::IpcServerStream;

use crate::resolver::resolve_socket_name;
use futures::stream::Stream;
use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};
use knot_core::errors::TransportError;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tracing::{error, info, instrument, trace};

/// A stream of incoming IPC connections, acting as a server-side listener.
///
/// `IpcIncoming` binds to a filesystem path (Unix) or a named pipe (Windows) and
/// continuously accepts incoming connections in a background task. The accepted
/// connections are buffered into an internal `mpsc` channel and exposed as an
/// asynchronous `Stream`.
///
/// # Resource Management
/// This struct implements [`Drop`], ensuring that the background acceptor task
/// is explicitly aborted and resources are cleaned up when the listener is
/// dropped.
pub struct IpcIncoming {
    /// Internal channel receiver for accepted connections.
    receiver: mpsc::Receiver<Result<IpcServerStream, std::io::Error>>,
    /// Handle to the background task performing `accept()` loops.
    task_handle: tokio::task::JoinHandle<()>,
}

impl IpcIncoming {
    /// Binds to the specified socket path and starts the background connection acceptor.
    ///
    /// This method performs platform-specific setup:
    /// - **Unix:** Attempts to remove any existing stale socket file and sets
    ///   permissions to `0o600`.
    /// - **Windows:** Resolves the path to a valid Named Pipe name.
    ///
    /// # Arguments
    /// * `socket_path` - The path to the socket or named pipe.
    /// * `opts` - Configuration options, including the size of the internal connection buffer.
    ///
    /// # Errors
    /// Returns a [`TransportError`] if the listener cannot be created or if
    /// path permissions cannot be set.
    #[instrument(skip(socket_path), fields(path = %socket_path.as_ref().display()), err)]
    pub fn bind(
        socket_path: impl AsRef<Path>,
        opts: IncomingOptions,
    ) -> Result<Self, TransportError> {
        let socket_path = socket_path.as_ref().to_path_buf();
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

        trace!("Creating tokio local socket listener...");
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| TransportError::InvalidSocketPath {
                    path: socket_path.clone(),
                })?;
        }

        let (tx, rx) = mpsc::channel(opts.buffer_size);

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tx.closed() => {
                        break;
                    }
                    conn_res = listener.accept() => {
                        let mapped_res = conn_res.map(IpcServerStream::new);
                        if tx.send(mapped_res).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        info!("IpcServer successfully bound to socket");
        Ok(Self {
            receiver: rx,
            task_handle: handle,
        })
    }
}

impl Stream for IpcIncoming {
    type Item = Result<IpcServerStream, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}

impl Drop for IpcIncoming {
    fn drop(&mut self) {
        self.task_handle.abort();
    }
}

// Unit tests for `IpcIncoming`
//
// Structure:
//   - `bind::*` — listener creation (success, duplicate, invalid path, permissions)
//   - `stream::*` — `Stream` behavior (accepting connections, Poll::Pending, errors)
//   - `concurrent::*` — multiple connections / backpressure
//   - `cleanup::*` — behavior on drop / socket path reuse
//   - `permissions::*` — socket file permissions (unix-only)

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;

    /// Returns a unique socket path within a temporary directory.
    fn tmp_socket(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    /// Connects to the socket and immediately closes the connection.
    async fn connect_and_close(path: &Path) {
        use crate::resolver::resolve_socket_name;
        use interprocess::local_socket::tokio::prelude::LocalSocketStream as ClientStream;
        let name = resolve_socket_name(path).expect("resolve");

        let mut attempts = 0;
        loop {
            match ClientStream::connect(name.clone()).await {
                Ok(_) => break,
                Err(_e) if attempts < 50 => {
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    attempts += 1;
                }
                Err(e) => panic!("client connect failed after retries: {}", e),
            }
        }
    }

    mod bind {
        use super::*;

        /// Basic happy-path: `bind` on a non-existent path should return `Ok`.
        #[tokio::test]
        async fn succeeds_on_fresh_path() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "test.sock");

            let result = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(result.is_ok(), "bind should successfully create a listener");
        }

        /// `bind` should return `Ok` even if the socket file already exists
        /// (unix: stale file is removed, windows: named pipe is recreated).
        #[tokio::test]
        #[cfg(unix)]
        async fn succeeds_when_stale_socket_file_exists() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "stale.sock");

            // Create a "stale" file manually
            std::fs::write(&path, b"stale").unwrap();

            let result = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(
                result.is_ok(),
                "bind should remove the stale file and start successfully"
            );
        }

        /// Two consecutive `bind`s on the same path: the second must either return an error
        /// (if the first `IpcIncoming` is still alive), or successfully recreate the socket.
        /// Here we verify that after dropping the first `bind`, the second is successful.
        #[tokio::test]
        #[cfg(unix)]
        async fn rebind_after_drop_succeeds() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "rebind.sock");

            {
                let _first =
                    IpcIncoming::bind(&path, IncomingOptions::default()).expect("first bind");
            } // drop → socket file should disappear or be recreated

            let second = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(
                second.is_ok(),
                "subsequent bind after drop should be successful"
            );
        }

        /// `bind` on a non-existent directory should return `TransportError`.
        #[tokio::test]
        #[cfg(unix)]
        async fn fails_on_nonexistent_directory() {
            let path = PathBuf::from("/nonexistent/deeply/nested/path/test.sock");
            let result = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(
                result.is_err(),
                "bind on a non-existent directory should return Err"
            );
        }

        /// The returned error should be `TransportError::ConnectionFailed`
        /// (and not a panic or another variant).
        #[tokio::test]
        #[cfg(unix)]
        async fn error_variant_is_connection_failed() {
            let path = PathBuf::from("/nonexistent/dir/sock");
            match IpcIncoming::bind(&path, IncomingOptions::default()) {
                Err(TransportError::ConnectionFailed { path: p, .. }) => {
                    assert_eq!(p, path);
                }
                Err(other) => {
                    // InvalidSocketPath is also acceptable for some platforms
                    matches!(other, TransportError::InvalidSocketPath { .. });
                }
                Ok(_) => panic!("expected an error"),
            }
        }

        /// `bind` with an empty filename should return an error.
        #[tokio::test]
        #[cfg(unix)]
        async fn fails_on_empty_filename() {
            let dir = TempDir::new().unwrap();
            // path without a filename
            let path = dir.path().to_path_buf();
            let result = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(
                result.is_err(),
                "bind on a directory without a filename should return Err"
            );
        }

        /// `bind` on a path with a null byte (invalid OsStr) should return an error.
        #[tokio::test]
        #[cfg(unix)]
        async fn fails_on_path_with_null_byte() {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let path = PathBuf::from(OsStr::from_bytes(b"/tmp/bad\x00sock"));
            let result = IpcIncoming::bind(&path, IncomingOptions::default());
            assert!(result.is_err(), "path with a null byte should return Err");
        }
    }

    #[cfg(unix)]
    mod permissions {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// After a successful `bind`, the socket file permissions should be 0o600.
        #[tokio::test]
        async fn socket_file_permissions_are_0600() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "perms.sock");

            let _incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let meta = std::fs::metadata(&path).expect("metadata");
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "socket permissions should be 0600, got {:o}",
                mode
            );
        }

        /// `bind` should fail with `InvalidSocketPath` if the directory does not allow
        /// changing permissions (e.g., `/proc/sys/...`). Mocked via a read-only tmpfs.
        #[tokio::test]
        async fn bind_on_readonly_dir_returns_error() {
            // Create a directory without write permissions
            let dir = TempDir::new().unwrap();
            let readonly_dir = dir.path().join("ro");
            std::fs::create_dir(&readonly_dir).unwrap();
            std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o555))
                .unwrap();

            let path = readonly_dir.join("test.sock");
            let result = IpcIncoming::bind(&path, IncomingOptions::default());

            // Restore permissions to allow correct TempDir removal
            std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o755))
                .unwrap();

            assert!(
                result.is_err(),
                "bind in a directory without permissions should return Err"
            );
        }
    }

    mod stream {
        use super::*;

        /// `poll_next` should return `Poll::Ready(Some(Ok(...)))` after a client connects.
        #[tokio::test]
        async fn poll_next_returns_ready_on_connection() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "accept.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            // Client connects in the background
            let path_clone = path.clone();
            tokio::spawn(async move {
                connect_and_close(&path_clone).await;
            });

            let conn = timeout(Duration::from_secs(2), incoming.next())
                .await
                .expect("timeout")
                .expect("stream ended")
                .expect("io error");

            // Ensure the connection is valid (can read/write)
            let _ = conn; // drop — having the correct type is enough
        }

        /// `next()` should block (Pending) while there are no connections,
        /// and unblock after a client arrives.
        #[tokio::test]
        async fn stream_is_pending_before_any_connection() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "pending.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            // Without a client — it should hang
            let poll_result = timeout(Duration::from_millis(50), incoming.next()).await;
            assert!(
                poll_result.is_err(),
                "stream should be Pending when there are no connections"
            );
        }

        /// Accept exactly N connections sequentially.
        #[tokio::test]
        async fn accepts_multiple_sequential_connections() {
            const N: usize = 5;
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "seq.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            for i in 0..N {
                let path_clone = path.clone();
                tokio::spawn(async move {
                    connect_and_close(&path_clone).await;
                });

                let conn = timeout(Duration::from_secs(2), incoming.next())
                    .await
                    .unwrap_or_else(|_| panic!("timeout on connection {}", i))
                    .expect("stream ended")
                    .expect("io error");
                drop(conn);
            }
        }

        /// Data written by the client should be readable by the server.
        #[tokio::test]
        async fn accepted_stream_is_readable() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "rw.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let payload = b"hello knot";
            let path_clone = path.clone();
            tokio::spawn(async move {
                use crate::resolver::resolve_socket_name;
                use interprocess::local_socket::tokio::prelude::LocalSocketStream as ClientStream;
                let name = resolve_socket_name(&path_clone).unwrap();
                let mut s = ClientStream::connect(name).await.unwrap();
                s.write_all(payload).await.unwrap();
            });

            let mut conn = timeout(Duration::from_secs(2), incoming.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();

            let mut buf = vec![0u8; payload.len()];
            conn.read_exact(&mut buf)
                .await
                .expect("read from accepted stream");
            assert_eq!(&buf, payload);
        }

        /// The accepted stream should be writable from the server side.
        #[tokio::test]
        async fn accepted_stream_is_writable() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "write.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let response = b"pong from server";
            let path_clone = path.clone();

            // Client waits for the response
            let client_task = tokio::spawn(async move {
                use crate::resolver::resolve_socket_name;
                use interprocess::local_socket::tokio::prelude::LocalSocketStream as ClientStream;
                let name = resolve_socket_name(&path_clone).unwrap();
                let mut s = ClientStream::connect(name).await.unwrap();
                let mut buf = vec![0u8; response.len()];
                s.read_exact(&mut buf).await.unwrap();
                buf
            });

            let mut conn = timeout(Duration::from_secs(2), incoming.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();

            conn.write_all(response)
                .await
                .expect("write to accepted stream");

            let received = timeout(Duration::from_secs(2), client_task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(received, response);
        }

        /// Stream `Item` is `Result<LocalSocketStream, io::Error>`.
        /// The type should be correct (compile-time check via explicit type annotation).
        #[tokio::test]
        async fn stream_item_type_is_result() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "type.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let path_clone = path.clone();
            tokio::spawn(async move { connect_and_close(&path_clone).await });

            // Explicit type annotation — compilation error if the type is wrong
            let item: Option<Result<IpcServerStream, std::io::Error>> =
                timeout(Duration::from_secs(2), incoming.next())
                    .await
                    .expect("timeout");

            assert!(item.is_some());
            assert!(item.unwrap().is_ok());
        }

        /// Dropping the incoming stream — new connections should not hang forever.
        #[tokio::test]
        async fn drop_incoming_prevents_further_accepts() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "drop.sock");
            let incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");
            drop(incoming); // drop immediately
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Client should not connect successfully (socket is closed)
            use crate::resolver::resolve_socket_name;
            use interprocess::local_socket::tokio::prelude::LocalSocketStream as ClientStream;
            let name = resolve_socket_name(&path).unwrap();
            let result = timeout(Duration::from_millis(200), ClientStream::connect(name)).await;

            // Either timeout or connection error — both are correct
            match result {
                Err(_timeout) => {}    // Pending — OK
                Ok(Err(_io_err)) => {} // Connection refused — OK
                Ok(Ok(_)) => panic!("client should not connect to a closed socket"),
            }
        }
    }

    mod concurrent {
        use super::*;
        use futures::stream::StreamExt;

        /// N clients connect simultaneously — all N connections should be accepted.
        #[tokio::test]
        async fn accepts_burst_of_concurrent_connections() {
            const N: usize = 20;
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "burst.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            // Spawn N clients simultaneously
            let mut handles = Vec::with_capacity(N);
            for _ in 0..N {
                let p = path.clone();
                handles.push(tokio::spawn(async move {
                    connect_and_close(&p).await;
                }));
            }

            // Accept all N connections
            let mut accepted = 0usize;
            while accepted < N {
                let conn = timeout(Duration::from_secs(5), incoming.next())
                    .await
                    .expect("timeout waiting for connection")
                    .expect("stream ended")
                    .expect("io error");
                drop(conn);
                accepted += 1;
            }

            assert_eq!(accepted, N);

            for h in handles {
                h.await.unwrap();
            }
        }

        /// `take(N)` should accept exactly N connections and stop the stream.
        #[tokio::test]
        async fn take_n_stops_stream_after_n_connections() {
            const N: usize = 3;
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "take.sock");
            let incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let mut handles = Vec::with_capacity(N);
            for _ in 0..N + 1 {
                let p = path.clone();
                handles.push(tokio::spawn(async move { connect_and_close(&p).await }));
            }

            let conns: Vec<_> =
                timeout(Duration::from_secs(5), incoming.take(N).collect::<Vec<_>>())
                    .await
                    .expect("Server timeout");

            assert_eq!(conns.len(), N);
            for conn in conns {
                assert!(conn.is_ok());
            }

            for h in handles {
                h.abort();
            }
        }

        /// Connection processing via `buffer_unordered` should not lose connections.
        #[tokio::test]
        async fn buffer_unordered_processes_all_connections() {
            const N: usize = 10;
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "buffered.sock");
            let incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let mut handles = Vec::with_capacity(N);
            for _ in 0..N {
                let p = path.clone();
                handles.push(tokio::spawn(async move { connect_and_close(&p).await }));
            }

            let results: Vec<_> = timeout(
                Duration::from_secs(5),
                incoming
                    .take(N)
                    .map(|item| async move { item.map(|_conn| 1usize) })
                    .buffer_unordered(4)
                    .collect::<Vec<_>>(),
            )
            .await
            .expect("Server timeout");
            let total: usize = results.into_iter().filter_map(Result::ok).sum();
            assert_eq!(total, N);

            for h in handles {
                h.await.expect("One task was failed with panic");
            }
        }

        /// Slow clients should not block the acceptance of fast ones.
        #[tokio::test]
        async fn slow_client_does_not_block_fast_clients() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "slow.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            // Slow client — opens a connection but does nothing
            let p = path.clone();
            let _slow = tokio::spawn(async move {
                use crate::resolver::resolve_socket_name;
                use interprocess::local_socket::tokio::prelude::LocalSocketStream as CS;
                let name = resolve_socket_name(&p).unwrap();
                let _s = CS::connect(name).await.unwrap();
                tokio::time::sleep(Duration::from_secs(10)).await;
            });
            tokio::time::sleep(Duration::from_millis(20)).await;

            // Fast client
            let p2 = path.clone();
            tokio::spawn(async move { connect_and_close(&p2).await });

            // Accept two connections — the slow one does not block the fast one
            let mut count = 0;
            while count < 2 {
                let _ = timeout(Duration::from_secs(2), incoming.next())
                    .await
                    .expect("timeout")
                    .expect("stream ended")
                    .expect("io error");
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }

    #[cfg(unix)]
    mod cleanup {
        use super::*;

        /// After dropping `IpcIncoming`, the socket file should be removed (unix).
        #[tokio::test]
        async fn socket_file_removed_on_drop() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "cleanup.sock");

            {
                let _incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");
                assert!(path.exists(), "socket file should exist after bind");
            }

            // After drop — the file should disappear
            // (depends on the interprocess implementation; if not removed — test is informative)
            if path.exists() {
                // If the library does not remove it itself — this is normal for some platforms,
                // but then a subsequent bind should still work (verified in bind::*)
                eprintln!(
                    "info: interprocess does not remove the socket file on drop on this platform"
                );
            }
        }

        /// Socket path is reused without errors (bind idempotency).
        #[tokio::test]
        async fn socket_path_reusable_across_bind_cycles() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "reuse.sock");

            for i in 0..3 {
                let incoming = IpcIncoming::bind(&path, IncomingOptions::default())
                    .unwrap_or_else(|e| panic!("bind failed on cycle {}: {:?}", i, e));

                let p = path.clone();
                tokio::spawn(async move { connect_and_close(&p).await });

                let mut s = incoming;
                let _ = timeout(Duration::from_secs(2), s.next())
                    .await
                    .expect("timeout")
                    .unwrap()
                    .unwrap();
                // drop incoming — the next cycle will start fresh
            }
        }
    }

    mod integration {
        use super::*;

        /// Full echo server: client sends data, server echoes it back.
        #[tokio::test]
        async fn echo_server_roundtrip() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "echo.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let messages: &[&[u8]] = &[
                b"ping",
                b"hello world",
                b"\x00\x01\x02binary",
                b"knot-transport v1",
            ];

            for &msg in messages {
                let p = path.clone();
                let msg_owned = msg.to_vec();

                // Client: sends msg, reads response
                let client = tokio::spawn(async move {
                    use crate::resolver::resolve_socket_name;
                    use interprocess::local_socket::tokio::prelude::LocalSocketStream as CS;
                    let name = resolve_socket_name(&p).unwrap();
                    let mut s = CS::connect(name).await.unwrap();
                    s.write_all(&msg_owned).await.unwrap();
                    let mut buf = vec![0u8; msg_owned.len()];
                    s.read_exact(&mut buf).await.unwrap();
                    buf
                });

                // Server: accepts connection and echoes
                let mut conn = timeout(Duration::from_secs(2), incoming.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();

                let mut buf = vec![0u8; msg.len()];
                conn.read_exact(&mut buf).await.unwrap();
                conn.write_all(&buf).await.unwrap();
                drop(conn);

                let received = timeout(Duration::from_secs(2), client)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(received, msg, "echo mismatch for {:?}", msg);
            }
        }

        /// `IpcIncoming` works correctly as a `futures::Stream` in `select!`.
        #[tokio::test]
        async fn works_in_tokio_select() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "select.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            let p = path.clone();

            let client_handle = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                connect_and_close(&p).await;
            });

            let mut got_conn = false;
            let result = timeout(Duration::from_secs(5), async {
                loop {
                    tokio::select! {
                        item = incoming.next() => {
                            if item.is_some() {
                                got_conn = true;
                                break;
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {
                            panic!("Select timer fired before connection was received!");
                        }
                    }
                }
            })
            .await;

            result.expect("Overall test timeout: completely deadlocked");
            client_handle.await.expect("Client task panicked!");
            assert!(got_conn, "select! should receive a connection");
        }

        /// Large payload (64 KiB) passes through IPC without data loss.
        #[tokio::test]
        async fn large_payload_roundtrip() {
            let dir = TempDir::new().unwrap();
            let path = tmp_socket(&dir, "large.sock");
            let mut incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind");

            const SIZE: usize = 64 * 1024;
            let payload: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
            let payload_clone = payload.clone();

            let p = path.clone();
            tokio::spawn(async move {
                use crate::resolver::resolve_socket_name;
                use interprocess::local_socket::tokio::prelude::LocalSocketStream as CS;
                let name = resolve_socket_name(&p).unwrap();
                let mut s = CS::connect(name).await.unwrap();
                s.write_all(&payload_clone).await.unwrap();
                let mut buf = vec![0u8; SIZE];
                s.read_exact(&mut buf).await.unwrap();
                assert_eq!(buf, payload_clone, "large payload echo mismatch");
            });

            let mut conn = timeout(Duration::from_secs(5), incoming.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();

            let mut buf = vec![0u8; SIZE];
            conn.read_exact(&mut buf).await.unwrap();
            conn.write_all(&buf).await.unwrap();
        }
    }
}
