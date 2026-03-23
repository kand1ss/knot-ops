use knot_core::errors::TransportError;
use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::{
        Message, MessageKind,
        daemon::{DaemonRequest, DaemonResponse, DaemonEvent},
    },
    transport::{
        MessageTransport, RawTransport, Server,
        ipc::{IpcServer, IpcTransport},
    },
};
use rstest::*;
use std::marker::PhantomData;
use std::path::PathBuf;
use tokio::task::JoinHandle;

fn test_socket_path(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let thread_id = std::thread::current().id();
    path.push(format!("knot-test-{}-{:?}.sock", suffix, thread_id));
    path
}

async fn spawn_echo_server<Cod>(socket_path: PathBuf) -> JoinHandle<()>
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let server = IpcServer::bind(socket_path).await.unwrap();
        println!("server spawned");
        let transport: MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod> =
            server.accept().await.unwrap().to_messaged();
        println!("received transport");

        while let Ok(msg) = transport.recv().await {
            match msg.kind {
                MessageKind::Request(req) => {
                    let response = match req {
                        DaemonRequest::Down => {
                            let _ = transport
                                .send(Message::response(msg.id, DaemonResponse::Ok))
                                .await;
                            break;
                        },
                        DaemonRequest::Status => DaemonResponse::Status {
                            services: Vec::new(),
                        },
                    };
                    let _ = transport
                        .send(Message::response(msg.id, response))
                        .await;
                },
                MessageKind::Event(ev) => {
                    let _ = transport.send(Message::event(ev)).await;
                },
                MessageKind::Response(_) => {}
            }
        }
    })
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
enum TestRequest {
    Ping(i32),
}
#[derive(Serialize, Deserialize, Debug)]
enum TestResponse {
    Pong(i32),
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
            let transport: MessageTransport<IpcTransport, TestRequest, TestResponse, DaemonEvent, Cod> =
                raw.to_messaged();

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
            let client: MessageTransport<IpcTransport, TestRequest, TestResponse, DaemonEvent, Cod> =
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
    println!("server spawned 2");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();
    println!("received client");

    println!("waiting response");
    let response = client.request(DaemonRequest::Down).await.unwrap();

    assert!(matches!(response, DaemonResponse::Ok));

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

    let client: MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();

    for i in 0..5 {
        let response = client.request(DaemonRequest::Status).await.unwrap();

        assert!(
            matches!(response, DaemonResponse::Status { services: _ }),
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

    let result: Result<MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod>, _> =
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

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn test_shutdown_message<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = test_socket_path("shutdown");
    let server = spawn_echo_server::<Cod>(path.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client: MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();

    let response = client.request(DaemonRequest::Down).await.unwrap();

    assert!(matches!(response, DaemonResponse::Ok));

    tokio::time::timeout(std::time::Duration::from_secs(1), server)
        .await
        .unwrap()
        .ok();
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

    let client: MessageTransport<IpcTransport, DaemonRequest, DaemonResponse, DaemonEvent, Cod> =
        IpcTransport::connect(path).await.unwrap().to_messaged();

    drop(server);

    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client.request(DaemonRequest::Status).await;

    assert!(matches!(
        result,
        Err(TransportError::ConnectionClosed) | Err(TransportError::Io { .. })
    ));
}
