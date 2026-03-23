use knot_core::errors::TransportError;
use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::{Message, MessageKind},
    transport::{
        MessageTransport, RawTransport, Server,
        ipc::{IpcServer, IpcTransport},
    },
};
use rstest::*;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::path::PathBuf;
use tokio::task::JoinHandle;

fn test_socket_path(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let thread_id = std::thread::current().id();
    path.push(format!("knot-test-{}-{:?}.sock", suffix, thread_id));
    path
}

#[derive(Serialize, Deserialize, Debug)]
enum TestRequest {
    Ping(i32),
}
#[derive(Serialize, Deserialize, Debug)]
enum TestResponse {
    Pong(i32),
}
#[derive(Serialize, Deserialize, Debug)]
enum TestEvent {
    Event(i32),
}

type TestTransport<Transport, Codec> =
    MessageTransport<Transport, TestRequest, TestResponse, TestEvent, Codec>;
type TestMessage = Message<TestRequest, TestResponse, TestEvent>;

async fn spawn_echo_server<Cod>(socket_path: PathBuf) -> JoinHandle<()>
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let server = IpcServer::bind(socket_path).await.unwrap();
        let transport: TestTransport<IpcTransport, Cod> =
            server.accept().await.unwrap().to_messaged();

        while let Ok(msg) = transport.recv().await {
            match msg.kind {
                MessageKind::Request(req) => {
                    let TestRequest::Ping(val) = req;
                    transport
                        .send(TestMessage::response(msg.id, TestResponse::Pong(val)))
                        .await
                        .ok();
                }
                MessageKind::Event(ev) => {
                    let _ = transport.send(Message::event(ev)).await;
                }
                MessageKind::Response(_) => {}
            }
        }
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
    let path = test_socket_path("isolation");
    let server_path = path.clone();

    tokio::spawn(async move {
        let server = IpcServer::bind(server_path).await.unwrap();

        while let Ok(raw) = server.accept().await {
            let transport: TestTransport<IpcTransport, Cod> = raw.to_messaged();

            tokio::spawn(async move {
                while let Ok(msg) = transport.recv().await {
                    if let MessageKind::Request(TestRequest::Ping(val)) = msg.kind {
                        let response = Message::response(msg.id, TestResponse::Pong(val));
                        let _ = transport.send(response).await;
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
            let client: TestTransport<IpcTransport, Cod> =
                IpcTransport::connect(p).await.unwrap().to_messaged();

            for j in 0..5 {
                let unique_val = i * 100 + j;
                let response = client
                    .request(TestRequest::Ping(unique_val))
                    .await
                    .expect("Request failed");

                let TestResponse::Pong(received_val) = response;
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
    let path = test_socket_path("connect");
    let server = spawn_echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: TestTransport<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    let response = client.request(TestRequest::Ping(1)).await.unwrap();

    assert!(matches!(response, TestResponse::Pong(1)));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_connect_send_event<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = test_socket_path("event");
    let server = spawn_echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: TestTransport<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    client
        .send(TestMessage::event(TestEvent::Event(0)))
        .await
        .expect("failed event sending");
    let response = client.recv().await.expect("failed receiving response");

    assert!(matches!(
        response.kind,
        MessageKind::Event(TestEvent::Event(0))
    ));
    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_multiple_requests<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = test_socket_path("multiple");
    let server = spawn_echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: TestTransport<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();

    for i in 0..5 {
        let response = client.request(TestRequest::Ping(i)).await.unwrap();

        assert!(
            matches!(response, TestResponse::Pong(_)),
            "request {} failed",
            i
        );
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
    let path = test_socket_path("no-server");

    let result: Result<TestTransport<IpcTransport, Cod>, _> =
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
    let path = test_socket_path("stale_shutdown");

    let mut _server = IpcServer::bind(path.clone()).await.unwrap();
    assert!(path.exists());

    _server.shutdown().await;
    assert!(!path.exists());
}

#[tokio::test]
#[cfg(not(windows))]
async fn test_server_bind_cleans_stale_socket_drop() {
    let path = test_socket_path("stale_drop");

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
    let path = test_socket_path("drop");
    let server = IpcServer::bind(path.clone()).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: TestTransport<IpcTransport, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();

    drop(server);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client.request(TestRequest::Ping(0)).await;

    assert!(matches!(
        result,
        Err(TransportError::ConnectionClosed) | Err(TransportError::Io { .. })
    ));
}
