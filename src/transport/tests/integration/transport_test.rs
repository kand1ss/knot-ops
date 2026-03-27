use async_trait::async_trait;
use knot_core::errors::TransportError;
use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::{Message, MessageContext, MessageKind, MetadataMap},
    middleware::{Inbound, Outbound, traits::Middleware},
    transport::{
        MAX_MESSAGE_SIZE, MessageTransport, RawTransport, Server, TransportSpec,
        ipc::{IpcServer, IpcTransport},
    },
};
use rstest::*;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::task::JoinHandle;

fn sock(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let thread_id = std::thread::current().id();
    path.push(format!("knot-test-{}-{:?}.sock", suffix, thread_id));
    path
}

#[derive(Serialize, Deserialize, Debug)]
enum Req {
    Ping(i32),
}
#[derive(Serialize, Deserialize, Debug)]
enum Res {
    Pong(i32),
}
#[derive(Serialize, Deserialize, Debug)]
enum Ev {
    Event(i32),
}

#[derive(Debug)]
struct Spec<C: MessageCodec<Raw = Vec<u8>>>(PhantomData<C>);
impl<C: MessageCodec<Raw = Vec<u8>>> TransportSpec for Spec<C> {
    type Req = Req;
    type Res = Res;
    type Ev = Ev;
    type C = C;
}

type Trans<T, C> = MessageTransport<T, Spec<C>>;
type Msg = Message<Req, Res, Ev>;

#[derive(Debug, Default)]
pub struct Counter {
    pub(crate) request_counter: AtomicUsize,
    pub(crate) response_counter: AtomicUsize,
    pub(crate) send_counter: AtomicUsize,
    pub(crate) recv_counter: AtomicUsize,
}

#[derive(Debug, Clone)]
pub struct CounterMiddleware<C: MessageCodec<Raw = Vec<u8>> + Debug>(Arc<Counter>, PhantomData<C>);
#[async_trait]
impl<C> Middleware<IpcTransport, Spec<C>> for CounterMiddleware<C>
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    async fn on_recv(
        &self,
        msg: &Msg,
        next: Inbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        if msg.is_response() {
            self.0.response_counter.fetch_add(1, Ordering::Relaxed);
        }
        self.0.recv_counter.fetch_add(1, Ordering::Relaxed);

        next.run(msg).await
    }

    async fn on_send(
        &self,
        msg: &mut Msg,
        next: Outbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        if msg.is_request() {
            self.0.request_counter.fetch_add(1, Ordering::Relaxed);
        }
        self.0.send_counter.fetch_add(1, Ordering::Relaxed);

        msg.set_meta("counter_middleware", "true");
        next.run(msg).await
    }
}

#[derive(Clone, Debug)]
struct RecvCounter<C: MessageCodec<Raw = Vec<u8>> + Debug>(Arc<AtomicUsize>, PhantomData<C>);

#[async_trait]
impl<C> Middleware<IpcTransport, Spec<C>> for RecvCounter<C>
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    async fn on_recv(
        &self,
        msg: &Msg,
        next: Inbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        next.run(msg).await
    }

    async fn on_send(
        &self,
        msg: &mut Msg,
        next: Outbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        next.run(msg).await
    }
}

/// Прерывает цепочку — не вызывает next, возвращает TransportError
#[derive(Clone, Debug)]
struct AbortMiddleware<C: MessageCodec<Raw = Vec<u8>> + Debug>(PhantomData<C>);

#[async_trait]
impl<C> Middleware<IpcTransport, Spec<C>> for AbortMiddleware<C>
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    async fn on_recv(
        &self,
        _msg: &Msg,
        _next: Inbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        Err(TransportError::ConnectionClosed)
    }

    async fn on_send(
        &self,
        _msg: &mut Msg,
        _next: Outbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        Err(TransportError::ConnectionClosed)
    }
}

/// Добавляет metadata-ключ на каждый исходящий/входящий message
#[derive(Clone, Debug)]
struct TagMiddleware<C: MessageCodec<Raw = Vec<u8>> + Debug> {
    tag: &'static str,
    _p: PhantomData<C>,
}

#[async_trait]
impl<C> Middleware<IpcTransport, Spec<C>> for TagMiddleware<C>
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    async fn on_recv(
        &self,
        msg: &Msg,
        next: Inbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        next.run(msg).await
    }

    async fn on_send(
        &self,
        msg: &mut Msg,
        next: Outbound<'_, IpcTransport, Spec<C>>,
    ) -> Result<(), TransportError> {
        msg.set_meta(self.tag, "1");
        next.run(msg).await
    }
}

async fn echo_server<Cod>(socket_path: PathBuf) -> JoinHandle<()>
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let server = IpcServer::bind(socket_path).await.unwrap();
        let transport: Trans<IpcTransport, Cod> = server.accept().await.unwrap().to_messaged();

        transport
            .serve_with(
                async |mut ctx: MessageContext<'_, IpcTransport, Spec<Cod>>| match ctx.kind() {
                    MessageKind::Request(req) => {
                        let Req::Ping(val) = req;
                        let mut current_val = *val;
                        let mut metadata = MetadataMap::new();

                        if let Some(metadata_val) = ctx.get_meta("increment") {
                            if let Ok(inc) = metadata_val.parse::<i32>() {
                                current_val += inc;
                                metadata.insert_str("incremented", "true");
                            }
                        }

                        ctx.reply(Res::Pong(current_val), Some(metadata)).await
                    }
                    MessageKind::Event(ev) => {
                        let Ev::Event(val) = ev;
                        let mut metadata = MetadataMap::new();

                        if let Some(metadata_val) = ctx.get_meta("metadata") {
                            if let Ok(inc) = metadata_val.parse::<bool>() {
                                if inc {
                                    metadata.insert_str("metadata", "true");
                                }
                            }
                        }

                        let message = Msg::event(Ev::Event(*val)).with_metadata(metadata);
                        ctx.emit(message).await
                    }
                    MessageKind::Response(_) => Ok(()),
                },
            )
            .await;
    })
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_concurrent_clients_isolation<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("isolation");
    let server_path = path.clone();

    tokio::spawn(async move {
        let server = IpcServer::bind(server_path).await.unwrap();

        while let Ok(raw) = server.accept().await {
            let transport: Trans<IpcTransport, Cod> = raw.to_messaged();

            tokio::spawn(async move {
                while let Ok(mut msg) = transport.recv().await {
                    if let MessageKind::Request(Req::Ping(val)) = msg.kind() {
                        msg.reply(Res::Pong(*val), None).await.ok();
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut handles = Vec::new();

    for i in 0..10 {
        let p = path.clone();
        let handle = tokio::spawn(async move {
            let client: Trans<IpcTransport, Cod> =
                IpcTransport::connect(p).await.unwrap().to_messaged();

            for j in 0..5 {
                let unique_val = i * 100 + j;
                let response = client
                    .request(Req::Ping(unique_val), 10, None)
                    .await
                    .expect("Request failed");

                let Res::Pong(received_val) = response;
                assert_eq!(received_val, unique_val);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("Client task panicked");
    }
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_connect_and_request<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("connect");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();
    let response = client.request(Req::Ping(1), 10, None).await.unwrap();

    assert!(matches!(response, Res::Pong(1)));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_request_timeout<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("timeout_test");

    let server_path = path.clone();
    let server = tokio::spawn(async move {
        let listener = IpcServer::bind(server_path).await.unwrap();
        let _raw_transport = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();
    let result = client.request(Req::Ping(1), 1, None).await;

    assert!(
        matches!(result, Err(TransportError::Timeout { .. })),
        "Expected TransportError::Timeout, but got: {:?}",
        result
    );
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_request_with_metadata<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("request_metadata");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();
    let request = Req::Ping(1);
    let mut metadata = MetadataMap::with_capacity(1);
    metadata.insert_str("increment", "4");

    let ctx = client
        .request_full(request, 10, Some(metadata))
        .await
        .unwrap();
    assert_eq!(ctx.get_meta("incremented"), Some("true"));

    let (msg, _) = ctx.into_parts();
    let (_, response) = msg.into_response().unwrap();
    assert!(matches!(response, Res::Pong(5)));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_request_with_middleware<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + Debug + 'static,
{
    let path = sock("request_metadata");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let counter = Arc::new(Counter::default());
    let cm = CounterMiddleware(counter.clone(), PhantomData);
    let mut client: Trans<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    client.add_middleware(cm).await;

    let request = Req::Ping(1);
    let response = client.request(request, 10, None).await.unwrap();

    assert!(matches!(response, Res::Pong(1)));
    server.abort();

    assert_eq!(counter.request_counter.load(Ordering::Relaxed), 1);
    assert_eq!(counter.response_counter.load(Ordering::Relaxed), 1);
    assert_eq!(counter.send_counter.load(Ordering::Relaxed), 1);
    assert_eq!(counter.recv_counter.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_connect_send_event<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("event");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();
    client
        .send(Msg::event(Ev::Event(0)))
        .await
        .expect("failed event sending");
    let response = client.recv().await.expect("failed receiving response");
    let (message, _) = response.into_parts();

    assert!(matches!(message.kind, MessageKind::Event(Ev::Event(0))));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_send_with_metadata<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("send_metadata");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();
    let mut message = Msg::event(Ev::Event(1));
    message.set_meta("metadata", "true");

    client.send(message).await.unwrap();
    let ctx = client.recv().await.unwrap();
    assert_eq!(ctx.get_meta("metadata"), Some("true"));

    let (msg, _) = ctx.into_parts();
    let (_, response) = msg.into_event().unwrap();
    assert!(matches!(response, Ev::Event(1)));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_send_with_middleware<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + Debug + 'static,
{
    let path = sock("send_metadata");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let counter = Arc::new(Counter::default());
    let cm = CounterMiddleware(counter.clone(), PhantomData);
    let mut client: Trans<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    client.add_middleware(cm).await;
    let message = Msg::event(Ev::Event(1));

    client.send(message).await.unwrap();
    let ctx = client.recv().await.unwrap();
    let (msg, _) = ctx.into_parts();
    let (_, response) = msg.into_event().unwrap();
    assert!(matches!(response, Ev::Event(1)));
    server.abort();

    assert_eq!(counter.request_counter.load(Ordering::Relaxed), 0);
    assert_eq!(counter.response_counter.load(Ordering::Relaxed), 0);
    assert_eq!(counter.send_counter.load(Ordering::Relaxed), 1);
    assert_eq!(counter.recv_counter.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_recv_with_middleware<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + Debug + 'static,
{
    let path = sock("send_metadata");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let counter = Arc::new(Counter::default());
    let cm = CounterMiddleware(counter.clone(), PhantomData);
    let mut client: Trans<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    let message = Msg::event(Ev::Event(1));

    client.send(message).await.unwrap();
    client.add_middleware(cm).await;
    let ctx = client.recv().await.unwrap();

    let (msg, _) = ctx.into_parts();
    let (_, response) = msg.into_event().unwrap();
    assert!(matches!(response, Ev::Event(1)));
    server.abort();

    assert_eq!(counter.request_counter.load(Ordering::Relaxed), 0);
    assert_eq!(counter.response_counter.load(Ordering::Relaxed), 0);
    assert_eq!(counter.send_counter.load(Ordering::Relaxed), 0);
    assert_eq!(counter.recv_counter.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_multiple_requests<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("multiple");
    let server = echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();

    for i in 0..5 {
        let response = client.request(Req::Ping(i), 10, None).await.unwrap();

        assert!(matches!(response, Res::Pong(_)), "request {} failed", i);
    }

    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_connect_fails_when_no_server<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + std::fmt::Debug + 'static,
{
    let path = sock("no-server");

    let result: Result<Trans<IpcTransport, Cod>, _> =
        IpcTransport::connect(path).await.map(|t| t.to_messaged());

    assert!(
        matches!(result, Err(TransportError::ConnectionFailed { .. })),
        "Expected specific error, but got: {:?}",
        result
    );
}

#[tokio::test]
#[cfg(not(windows))]
async fn test_server_bind_cleans_stale_socket_shutdown() {
    let path = sock("stale_shutdown");

    let mut _server = IpcServer::bind(path.clone()).await.unwrap();
    assert!(path.exists());

    _server.shutdown().await;
    assert!(!path.exists());
}

#[tokio::test]
#[cfg(not(windows))]
async fn test_server_bind_cleans_stale_socket_drop() {
    let path = sock("stale_drop");

    {
        std::fs::write(&path, b"stale").unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(path.exists());
    {
        let _server = IpcServer::bind(path.clone()).await.unwrap();
    }
    assert!(!path.exists());
}

use tokio::time::Duration;

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_connection_closed_on_server_drop<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("drop");
    let server = IpcServer::bind(path.clone()).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();

    drop(server);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client.request(Req::Ping(0), 10, None).await;

    assert!(matches!(
        result,
        Err(TransportError::ConnectionClosed) | Err(TransportError::Io { .. })
    ));
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_response_correlation_id_matches_request<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("corr_id");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let ctx = client.request_full(Req::Ping(7), 10, None).await.unwrap();
    let req_id = ctx.get().id();

    let (msg, _) = ctx.into_parts();
    let (resp_id, _payload) = msg.into_response().unwrap();
    assert_eq!(
        req_id, resp_id,
        "response id must correlate with request id"
    );
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_multiple_middleware_all_called<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    let path = sock("stack_mw");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let c1 = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::new(AtomicUsize::new(0));

    let mut client: Trans<IpcTransport, C> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    client
        .add_middleware(RecvCounter::<C>(c1.clone(), PhantomData))
        .await;
    client
        .add_middleware(RecvCounter::<C>(c2.clone(), PhantomData))
        .await;

    client.request(Req::Ping(0), 10, None).await.unwrap();

    assert_eq!(c1.load(Ordering::Relaxed), 1, "first middleware must fire");
    assert_eq!(c2.load(Ordering::Relaxed), 1, "second middleware must fire");
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_middleware_abort_on_send_propagates_error<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    let path = sock("abort_send");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client: Trans<IpcTransport, C> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    client
        .add_middleware(AbortMiddleware::<C>(PhantomData))
        .await;

    let result = client.send(Msg::event(Ev::Event(0))).await;
    assert!(result.is_err(), "aborted send must propagate Err");
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_request_full_returns_context_with_correct_payload<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("req_full");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, C> = IpcTransport::connect(path).await.unwrap().to_messaged();
    let ctx = client.request_full(Req::Ping(123), 10, None).await.unwrap();
    let (msg, _) = ctx.into_parts();
    let (_id, Res::Pong(v)) = msg.into_response().unwrap();
    assert_eq!(v, 123);
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_request_full_with_metadata_in_response<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    // Re-use spawn_echo_server logic that echoes "incremented" meta from existing tests,
    // but here we build a fresh server inline to avoid coupling to other test helpers.
    let path = sock("req_full_meta");
    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            let t: Trans<IpcTransport, C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(async |mut ctx: MessageContext<'_, IpcTransport, Spec<C>>| {
                if let MessageKind::Request(Req::Ping(v)) = ctx.kind() {
                    let mut meta = MetadataMap::new();
                    if ctx.get_meta("add_tag") == Some("yes") {
                        meta.insert_str("tag_added", "true");
                    }
                    ctx.reply(Res::Pong(*v), Some(meta)).await
                } else {
                    Ok(())
                }
            })
            .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let client: Trans<IpcTransport, C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let mut req_meta = MetadataMap::new();
    req_meta.insert_str("add_tag", "yes");

    let ctx = client
        .request_full(Req::Ping(0), 10, Some(req_meta))
        .await
        .unwrap();
    assert_eq!(
        ctx.get_meta("tag_added"),
        Some("true"),
        "server-set metadata must arrive in response context"
    );
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_pipelined_concurrent_requests_same_client<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("pipeline");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Arc<Trans<IpcTransport, C>> =
        Arc::new(IpcTransport::connect(path).await.unwrap().to_messaged());

    let mut handles = Vec::new();
    for val in 0_i32..20 {
        let c = client.clone();
        handles.push(tokio::spawn(async move {
            let Res::Pong(got) = c.request(Req::Ping(val), 10, None).await.unwrap();
            assert_eq!(got, val);
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    srv.abort();
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum BigReq {
    Payload(String),
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum BigRes {
    Echo(String),
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum BigEv {}

#[derive(Debug)]
struct BigSpec<C: MessageCodec<Raw = Vec<u8>>>(PhantomData<C>);
impl<C: MessageCodec<Raw = Vec<u8>>> TransportSpec for BigSpec<C> {
    type Req = BigReq;
    type Res = BigRes;
    type Ev = BigEv;
    type C = C;
}

type BigTrans<C> = MessageTransport<IpcTransport, BigSpec<C>>;

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_large_payload_512kb_round_trip<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("large_payload");

    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            let t: BigTrans<C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(
                async |mut ctx: MessageContext<'_, IpcTransport, BigSpec<C>>| {
                    if let MessageKind::Request(BigReq::Payload(s)) = ctx.kind() {
                        ctx.reply(BigRes::Echo(s.clone()), None).await
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let client: BigTrans<C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let big = "x".repeat(512 * 1024); // 512 KB
    let BigRes::Echo(got) = client
        .request(BigReq::Payload(big.clone()), 30, None)
        .await
        .unwrap();
    assert_eq!(
        got.len(),
        big.len(),
        "large payload must survive round-trip intact"
    );
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_large_payload_5mb_round_trip<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("large_payload");

    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            let t: BigTrans<C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(
                async |mut ctx: MessageContext<'_, IpcTransport, BigSpec<C>>| {
                    if let MessageKind::Request(BigReq::Payload(s)) = ctx.kind() {
                        ctx.reply(BigRes::Echo(s.clone()), None).await
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let client: BigTrans<C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let big = "x".repeat(5 * 1024 * 1024);
    let BigRes::Echo(got) = client
        .request(BigReq::Payload(big.clone()), 30, None)
        .await
        .unwrap();
    assert_eq!(
        got.len(),
        big.len(),
        "large payload must survive round-trip intact"
    );
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_very_large_payload_must_err<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("large_payload");

    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            let t: BigTrans<C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(
                async |mut ctx: MessageContext<'_, IpcTransport, BigSpec<C>>| {
                    if let MessageKind::Request(BigReq::Payload(s)) = ctx.kind() {
                        ctx.reply(BigRes::Echo(s.clone()), None).await
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let client: BigTrans<C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let big = "x".repeat(MAX_MESSAGE_SIZE + 1);
    client
        .request(BigReq::Payload(big.clone()), 30, None)
        .await
        .unwrap_err();
    srv.abort();
}

#[tokio::test]
#[cfg(not(windows))]
async fn test_rebind_after_shutdown_succeeds() {
    let path = sock("rebind");

    {
        let mut srv = IpcServer::bind(path.clone()).await.unwrap();
        srv.shutdown().await;
        assert!(!path.exists(), "socket must be cleaned up after shutdown");
    }

    // Second bind on same path must succeed
    let _srv2 = IpcServer::bind(path.clone())
        .await
        .expect("second bind after shutdown must succeed");
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_event_without_metadata_has_no_extra_keys<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("ev_no_meta");
    let srv = echo_server::<C>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, C> = IpcTransport::connect(path).await.unwrap().to_messaged();
    client.send(Msg::event(Ev::Event(0))).await.unwrap();
    let ctx = client.recv().await.unwrap();

    // Server echoes event; it must not inject unexpected metadata
    assert!(
        ctx.get_meta("metadata").is_none(),
        "echoed event must not have spurious 'metadata' key"
    );
    assert!(ctx.get_meta("increment").is_none());
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_middleware_mutation_visible_on_server<C>(#[case] _m: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Debug + Send + Sync + 'static,
{
    // Server echoes back any metadata it received under "echo_mw_tag"
    let path = sock("mw_mutate");
    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            let t: Trans<IpcTransport, C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(async |mut ctx: MessageContext<'_, IpcTransport, Spec<C>>| {
                if let MessageKind::Request(Req::Ping(v)) = ctx.kind() {
                    let mut meta = MetadataMap::new();
                    // Echo back what the middleware injected
                    let tag_val = ctx.get_meta("mw_tag").map(|s| s.to_string());
                    if let Some(val) = tag_val {
                        meta.insert_str("echo_mw_tag", val);
                    }
                    ctx.reply(Res::Pong(*v), Some(meta)).await
                } else {
                    Ok(())
                }
            })
            .await;
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client: Trans<IpcTransport, C> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    // TagMiddleware sets "mw_tag" = "1" on every outgoing message
    client
        .add_middleware(TagMiddleware::<C> {
            tag: "mw_tag",
            _p: PhantomData,
        })
        .await;

    let ctx = client.request_full(Req::Ping(0), 10, None).await.unwrap();
    assert_eq!(
        ctx.get_meta("echo_mw_tag"),
        Some("1"),
        "mutation by on_send middleware must be visible on the server side"
    );
    srv.abort();
}
