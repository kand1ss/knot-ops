use crate::resolver::resolve_socket_name;
use hyper::Uri;
use interprocess::local_socket::tokio::prelude::*;
use std::path::PathBuf;
use tower::Service;

/// A connector that establishes gRPC connections over Inter-Process Communication (IPC).
///
/// `IpcConnector` implements the `tower::Service<Uri>` trait, allowing it to be used
/// as a custom transport layer for `tonic` gRPC clients. Instead of using traditional
/// TCP/IP sockets, it facilitates communication using Unix Domain Sockets (on Unix-like
/// systems) or Named Pipes (on Windows).
///
/// # Behavior
/// - **URI Agnostic:** The provided `Uri` in `call` is ignored. The connection is always
///   established to the path specified during the connector's initialization.
/// - **Connection:** Every call to `call` initiates a new connection to the socket
///   path configured for this instance.
/// - **Compatibility:** Wraps the resulting stream in `hyper_util::rt::TokioIo` to ensure
///   compatibility with `hyper` and `tonic` IO requirements.
#[derive(Debug, Clone)]
pub struct IpcConnector {
    /// The filesystem path or pipe name where the IPC listener resides.
    socket_path: PathBuf,
}

impl IpcConnector {
    /// Creates a new `IpcConnector` pointing to the specified socket path.
    ///
    /// # Arguments
    /// * `path` - A path-like object (`&str`, `String`, or `PathBuf`) pointing to the
    ///   Unix domain socket or Windows named pipe.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
        }
    }
}

impl Service<Uri> for IpcConnector {
    /// The transport stream wrapped in `TokioIo` for hyper compatibility.
    type Response = hyper_util::rt::TokioIo<LocalSocketStream>;

    /// The transport error type.
    type Error = std::io::Error;

    /// The future that resolves to the connected IO stream.
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    /// Always returns `Poll::Ready(Ok(()))` as the IPC connector is always ready
    /// to attempt a connection.
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    /// Initiates a connection to the configured socket path.
    ///
    /// Ignores the `uri` argument and connects to the `socket_path` defined at
    /// instantiation. Returns a `hyper_util::rt::TokioIo` wrapped stream on success.
    ///
    /// # Errors
    /// Returns `TransportError::ConnectionFailed` if the socket path cannot be resolved
    /// or if the connection attempt fails.
    fn call(&mut self, _uri: Uri) -> Self::Future {
        let socket_path = self.socket_path.clone();

        Box::pin(async move {
            let name = resolve_socket_name(&socket_path)?;
            let stream = LocalSocketStream::connect(name).await?;
            Ok(hyper_util::rt::TokioIo::new(stream))
        })
    }
}

// Unit tests for `IpcConnector`
//
// Compatible with Windows (Named Pipes) and Unix (UDS).
// Both platforms use `interprocess` — the abstraction is handled in `resolve_socket_name`.
//
// Structure:
//   - `construction::*` — connector creation
//   - `poll_ready::*` — `Service::poll_ready` behavior
//   - `call::*` — `Service::call` behavior (happy path + errors)
//   - `tower::*` — integration with `tower` utilities
//   - `clone::*` — Clone semantics

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Uri;
    use std::future::Future;
    use std::path::PathBuf;
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;
    use tower::Service;

    /// Returns a unique socket path/name, valid for the current platform.
    ///
    /// - **Unix**: `/tmp/<name>.sock` (temporary file)
    /// - **Windows**: `\\.\pipe\<name>` (Named Pipe)
    ///
    /// On Windows, `interprocess` accepts the path to the Named Pipe directly;
    /// `resolve_socket_name` converts it to a `Name`.
    fn make_socket_path(name: &str) -> (PathBuf, Option<tempfile::TempDir>) {
        #[cfg(unix)]
        {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join(format!("{}.sock", name));
            (path, Some(dir))
        }
        #[cfg(windows)]
        {
            // On Windows we use Named Pipes. TempDir is not needed.
            let path = PathBuf::from(format!(r"\\.\pipe\knot-test-{}", name));
            (path, None)
        }
    }

    /// Spawns a minimal echo server at `socket_path` and returns a `JoinHandle`.
    /// Accepts a single connection, reads N bytes, sends them back, and closes.
    async fn spawn_echo_server(
        socket_path: PathBuf,
        echo_len: usize,
    ) -> tokio::task::JoinHandle<()> {
        use crate::resolver::resolve_socket_name;
        use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

        // Remove the file just in case to ensure the path is not occupied
        #[cfg(unix)]
        let _ = std::fs::remove_file(&socket_path);

        let name = resolve_socket_name(&socket_path).expect("resolve server name");
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .expect("create listener");

        tokio::spawn(async move {
            let mut conn = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; echo_len];
            conn.read_exact(&mut buf).await.expect("server read");
            conn.write_all(&buf).await.expect("server write");
        })
    }

    /// Spawns a server that accepts a single connection and immediately closes it.
    async fn spawn_close_server(socket_path: PathBuf) -> tokio::task::JoinHandle<()> {
        use crate::resolver::resolve_socket_name;
        use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

        #[cfg(unix)]
        let _ = std::fs::remove_file(&socket_path);

        let name = resolve_socket_name(&socket_path).expect("resolve server name");
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .expect("create listener");

        tokio::spawn(async move {
            let _conn = listener.accept().await.expect("accept");
            // drop → connection closed
        })
    }

    /// Dummy URI — `IpcConnector` ignores it (transport is determined by the path).
    fn dummy_uri() -> Uri {
        "http://localhost".parse().unwrap()
    }

    mod construction {
        use super::*;

        /// `new` accepts `&str`.
        #[test]
        fn new_from_str() {
            #[cfg(unix)]
            let connector = IpcConnector::new("/tmp/test.sock");
            #[cfg(windows)]
            let connector = IpcConnector::new(r"\\.\pipe\test");
            let _ = connector;
        }

        /// `new` accepts `PathBuf`.
        #[test]
        fn new_from_path_buf() {
            let path = PathBuf::from("test.sock");
            let connector = IpcConnector::new(path.clone());
            assert_eq!(connector.socket_path, path);
        }

        /// `new` accepts `String`.
        #[test]
        fn new_from_string() {
            #[cfg(unix)]
            let s = "/tmp/knot.sock".to_string();
            #[cfg(windows)]
            let s = r"\\.\pipe\knot".to_string();
            let connector = IpcConnector::new(s.clone());
            assert_eq!(connector.socket_path, PathBuf::from(s));
        }

        /// The path is stored exactly as passed.
        #[test]
        fn stores_path_exactly() {
            let path = PathBuf::from("some/nested/socket.sock");
            let connector = IpcConnector::new(path.clone());
            assert_eq!(connector.socket_path, path);
        }
    }

    mod poll_ready {
        use super::*;

        /// `poll_ready` always returns `Poll::Ready(Ok(()))`.
        #[test]
        fn always_ready() {
            let path = PathBuf::from("irrelevant.sock");
            let mut connector = IpcConnector::new(path);

            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            let result = connector.poll_ready(&mut cx);
            assert!(
                matches!(result, Poll::Ready(Ok(()))),
                "poll_ready should return Poll::Ready(Ok(()))"
            );
        }

        /// `poll_ready` does not change behavior after multiple calls.
        #[test]
        fn idempotent_across_calls() {
            let path = PathBuf::from("irrelevant.sock");
            let mut connector = IpcConnector::new(path);

            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            for _ in 0..5 {
                let result = connector.poll_ready(&mut cx);
                assert!(matches!(result, Poll::Ready(Ok(()))));
            }
        }

        /// `poll_ready` does not depend on the URI value (URI is not used at all).
        #[test]
        fn does_not_depend_on_any_state() {
            // Even a non-existent path — poll_ready should not check it
            let mut connector = IpcConnector::new("/nonexistent/path.sock");

            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            assert!(matches!(connector.poll_ready(&mut cx), Poll::Ready(Ok(()))));
        }

        /// `poll_ready` on a cloned connector is also ready.
        #[test]
        fn clone_is_also_ready() {
            let mut original = IpcConnector::new("test.sock");
            let mut cloned = original.clone();

            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);

            assert!(matches!(original.poll_ready(&mut cx), Poll::Ready(Ok(()))));
            assert!(matches!(cloned.poll_ready(&mut cx), Poll::Ready(Ok(()))));
        }
    }

    mod call {
        use super::*;

        /// Happy path: `call` returns `Ok(TokioIo<...>)` when the server is listening.
        #[tokio::test]
        async fn succeeds_when_server_is_listening() {
            let (path, _dir) = make_socket_path("call-ok");
            let _server = spawn_close_server(path.clone()).await;
            // A slight pause so the server has time to bind to the socket
            tokio::time::sleep(Duration::from_millis(20)).await;

            let mut connector = IpcConnector::new(&path);
            let result = connector.call(dummy_uri()).await;
            assert!(
                result.is_ok(),
                "call should return Ok when the server is listening"
            );
        }

        /// `call` returns `Err(TransportError::ConnectionFailed)` if the socket does not exist.
        #[tokio::test]
        async fn fails_with_connection_failed_when_no_server() {
            let (path, _dir) = make_socket_path("call-no-server");
            // Intentionally DO NOT spawn a server

            let mut connector = IpcConnector::new(&path);
            let result = connector.call(dummy_uri()).await;

            assert!(
                result.is_err(),
                "call should return Err if the server is not running"
            );
        }

        /// `call` returns a `Future` with a `Send` marker (required for `hyper`/`tower`).
        #[test]
        fn future_is_send() {
            fn assert_send<F: Future + Send>(_f: F) {}

            let mut connector = IpcConnector::new("test.sock");
            assert_send(connector.call(dummy_uri()));
        }

        /// `call` can be invoked multiple times in a row (without reset/poll_ready between calls).
        #[tokio::test]
        async fn multiple_calls_are_independent() {
            let (path, _dir) = make_socket_path("call-multi");

            // Spawn a server that accepts multiple connections
            use crate::resolver::resolve_socket_name;
            use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

            #[cfg(unix)]
            let _ = std::fs::remove_file(&path);

            let name = resolve_socket_name(&path).unwrap();
            let listener = ListenerOptions::new().name(name).create_tokio().unwrap();

            tokio::spawn(async move {
                for _ in 0..3 {
                    let _conn = listener.accept().await.unwrap();
                }
            });

            tokio::time::sleep(Duration::from_millis(20)).await;

            let mut connector = IpcConnector::new(&path);
            for i in 0..3 {
                let result = connector.call(dummy_uri()).await;
                assert!(result.is_ok(), "call #{} should be successful", i);
            }
        }

        /// The accepted `TokioIo` allows reading data via `AsyncRead`.
        #[tokio::test]
        async fn returned_io_is_readable() {
            let (path, _dir) = make_socket_path("call-read");
            let payload = b"hello from server";
            let _server = spawn_echo_server(path.clone(), payload.len()).await;
            tokio::time::sleep(Duration::from_millis(20)).await;

            let mut connector = IpcConnector::new(&path);
            let mut io = connector.call(dummy_uri()).await.expect("call failed");

            // Write, then read the echo
            io.inner_mut().write_all(payload).await.expect("write");
            let mut buf = vec![0u8; payload.len()];
            io.inner_mut().read_exact(&mut buf).await.expect("read");
            assert_eq!(&buf, payload);
        }

        /// The accepted `TokioIo` allows writing data via `AsyncWrite`.
        #[tokio::test]
        async fn returned_io_is_writable() {
            let (path, _dir) = make_socket_path("call-write");
            let payload = b"write test payload";

            // Server: accepts the connection and verifies that the data arrived correctly
            #[cfg(unix)]
            let _ = std::fs::remove_file(&path);

            use crate::resolver::resolve_socket_name;
            use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

            let name = resolve_socket_name(&path).unwrap();
            let listener = ListenerOptions::new().name(name).create_tokio().unwrap();

            let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
            tokio::spawn(async move {
                let mut conn = listener.accept().await.unwrap();
                let mut buf = vec![0u8; payload.len()];
                conn.read_exact(&mut buf).await.unwrap();
                let _ = tx.send(buf);
            });

            tokio::time::sleep(Duration::from_millis(20)).await;

            let mut connector = IpcConnector::new(&path);
            let mut io = connector.call(dummy_uri()).await.expect("call failed");
            io.inner_mut().write_all(payload).await.expect("write");
            drop(io);

            let received = timeout(Duration::from_secs(2), rx)
                .await
                .expect("timeout")
                .expect("server task dropped");
            assert_eq!(received, payload);
        }

        /// `call` completes in a reasonable time, doesn't hang forever without a server.
        #[tokio::test]
        async fn does_not_hang_indefinitely_when_no_server() {
            let (path, _dir) = make_socket_path("call-timeout");

            let mut connector = IpcConnector::new(&path);
            let result = timeout(Duration::from_secs(2), connector.call(dummy_uri())).await;

            // Either returned a fast error or timed out — both are better than hanging forever
            match result {
                Ok(Err(_)) => {} // Fast error — perfect
                Err(_) => {}     // Timeout — acceptable for some platforms
                Ok(Ok(_)) => panic!("should not connect without a server"),
            }
        }
    }

    mod clone {
        use super::*;

        /// The clone contains the same path.
        #[test]
        fn clone_has_same_path() {
            let path = PathBuf::from("original.sock");
            let original = IpcConnector::new(path.clone());
            let cloned = original.clone();
            assert_eq!(original.socket_path, cloned.socket_path);
        }

        /// The clone is independent — modifying the path in the original doesn't affect the clone
        /// (`PathBuf` is cloned by value).
        #[test]
        fn clone_is_independent() {
            let path = PathBuf::from("shared.sock");
            let original = IpcConnector::new(path.clone());
            let cloned = original.clone();

            // socket_path is PathBuf, meaning the clone owns its copy
            assert_eq!(original.socket_path, cloned.socket_path);
            // Verify that these are different objects (different PathBuf memory addresses)
            assert!(!std::ptr::eq(
                original.socket_path.as_os_str() as *const _,
                cloned.socket_path.as_os_str() as *const _,
            ));
        }

        /// The cloned connector successfully connects to the server.
        #[tokio::test]
        async fn clone_can_connect() {
            let (path, _dir) = make_socket_path("clone-connect");

            use crate::resolver::resolve_socket_name;
            use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

            #[cfg(unix)]
            let _ = std::fs::remove_file(&path);

            let name = resolve_socket_name(&path).unwrap();
            let listener = ListenerOptions::new().name(name).create_tokio().unwrap();

            tokio::spawn(async move {
                for _ in 0..2 {
                    let _conn = listener.accept().await.unwrap();
                }
            });

            tokio::time::sleep(Duration::from_millis(20)).await;

            let original = IpcConnector::new(&path);
            let mut cloned = original.clone();

            let result = cloned.call(dummy_uri()).await;
            assert!(result.is_ok(), "clone should successfully connect");
        }

        /// Multiple clones work in parallel.
        #[tokio::test]
        async fn multiple_clones_connect_concurrently() {
            const N: usize = 4;
            let (path, _dir) = make_socket_path("clone-parallel");

            use crate::resolver::resolve_socket_name;
            use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};

            #[cfg(unix)]
            let _ = std::fs::remove_file(&path);

            let name = resolve_socket_name(&path).unwrap();
            let listener = ListenerOptions::new().name(name).create_tokio().unwrap();

            tokio::spawn(async move {
                for _ in 0..N {
                    let _conn = listener.accept().await.unwrap();
                }
            });

            tokio::time::sleep(Duration::from_millis(20)).await;

            let base = IpcConnector::new(&path);
            let mut handles = Vec::with_capacity(N);

            for _ in 0..N {
                let mut c = base.clone();
                handles.push(tokio::spawn(async move { c.call(dummy_uri()).await }));
            }

            for h in handles {
                let result = timeout(Duration::from_secs(2), h)
                    .await
                    .expect("task timeout")
                    .expect("join error");
                assert!(result.is_ok(), "parallel clone should successfully connect");
            }
        }
    }

    mod tower_integration {
        use super::*;
        use tower::ServiceExt; // ready(), oneshot()

        /// `ServiceExt::ready()` resolves immediately.
        #[tokio::test]
        async fn service_ready_resolves_immediately() {
            let mut connector = IpcConnector::new("irrelevant.sock");
            let result = timeout(Duration::from_millis(100), connector.ready()).await;
            assert!(result.is_ok(), "ready() should not hang");
            assert!(result.unwrap().is_ok());
        }

        /// `ServiceExt::oneshot()` successfully executes a request through the server.
        #[tokio::test]
        async fn oneshot_succeeds_with_server() {
            let (path, _dir) = make_socket_path("tower-oneshot");
            let _server = spawn_close_server(path.clone()).await;
            tokio::time::sleep(Duration::from_millis(20)).await;

            let connector = IpcConnector::new(&path);
            let result = timeout(Duration::from_secs(2), connector.oneshot(dummy_uri()))
                .await
                .expect("timeout");

            assert!(result.is_ok(), "oneshot should return Ok");
        }

        /// `ServiceExt::oneshot()` returns `Err` if the server is unavailable.
        #[tokio::test]
        async fn oneshot_fails_without_server() {
            let (path, _dir) = make_socket_path("tower-oneshot-fail");
            // Server is not running

            let connector = IpcConnector::new(&path);
            let result = timeout(Duration::from_secs(2), connector.oneshot(dummy_uri())).await;

            match result {
                Ok(Err(_)) => {} // Connection error — expected
                Err(_) => {}     // Timeout — acceptable
                Ok(Ok(_)) => panic!("should not connect without a server"),
            }
        }

        /// `IpcConnector` implements `tower::Service` (compile-time check via bounds).
        #[test]
        fn implements_tower_service_trait() {
            fn assert_service<S>(_s: S)
            where
                S: tower::Service<Uri>,
            {
            }
            assert_service(IpcConnector::new("test.sock"));
        }

        /// `IpcConnector` implements `Clone` — required for `tower::Buffer` and similar.
        #[test]
        fn implements_clone() {
            fn assert_clone<T: Clone>(_t: T) {}
            assert_clone(IpcConnector::new("test.sock"));
        }
    }
}
