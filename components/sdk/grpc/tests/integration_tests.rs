#[cfg(test)]
mod echo {
    tonic::include_proto!("echo.v1");
}

#[cfg(test)]
mod integration_tests {
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::transport::{Endpoint, Server};
    use tonic::{Request, Response, Status, Streaming};

    use crate::echo::{
        EchoRequest, EchoResponse,
        echo_service_client::EchoServiceClient as EchoClient,
        echo_service_server::{EchoService, EchoServiceServer},
    };
    use knot_grpc::{IncomingOptions, IpcConnector, IpcIncoming};

    fn make_socket_path(name: &str) -> (PathBuf, Option<tempfile::TempDir>) {
        #[cfg(unix)]
        {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join(format!("{}.sock", name));
            (path, Some(dir))
        }
        #[cfg(windows)]
        {
            let path = PathBuf::from(format!(r"\\.\pipe\knot-grpc-test-{}", name));
            (path, None)
        }
    }

    #[derive(Debug, Default)]
    struct MyEchoService;

    #[tonic::async_trait]
    impl EchoService for MyEchoService {
        async fn unary_echo(
            &self,
            request: Request<EchoRequest>,
        ) -> Result<Response<EchoResponse>, Status> {
            Ok(Response::new(EchoResponse {
                message: request.into_inner().message,
            }))
        }

        type BidiEchoStream = ReceiverStream<Result<EchoResponse, Status>>;

        async fn bidi_echo(
            &self,
            request: Request<Streaming<EchoRequest>>,
        ) -> Result<Response<Self::BidiEchoStream>, Status> {
            let mut stream = request.into_inner();
            let (tx, rx) = mpsc::channel(128);

            tokio::spawn(async move {
                while let Ok(Some(req)) = stream.message().await {
                    let res = EchoResponse {
                        message: req.message,
                    };
                    if tx.send(Ok(res)).await.is_err() {
                        break;
                    }
                }
            });

            Ok(Response::new(ReceiverStream::new(rx)))
        }
    }

    async fn spawn_grpc_server(path: PathBuf) -> tokio::task::JoinHandle<()> {
        let incoming = IpcIncoming::bind(&path, IncomingOptions::default()).expect("bind failed");
        tokio::spawn(async move {
            Server::builder()
                .add_service(EchoServiceServer::new(MyEchoService))
                .serve_with_incoming(incoming)
                .await
                .expect("server failed");
        })
    }

    async fn create_grpc_client(path: &PathBuf) -> EchoClient<tonic::transport::Channel> {
        let connector = IpcConnector::new(path);
        let channel = Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(connector)
            .await
            .expect("client connect failed");

        EchoClient::new(channel)
    }

    async fn create_client_with_retry(path: &PathBuf) -> EchoClient<tonic::transport::Channel> {
        let mut client = None;
        for attempt in 0..10 {
            tokio::time::sleep(Duration::from_millis(10 * (1 << attempt.min(4)))).await;
            if let Ok(c) =
                tokio::time::timeout(Duration::from_millis(100), create_grpc_client(path)).await
            {
                client = Some(c);
                break;
            }
        }
        client.expect("Failed to connect to server after retries")
    }

    #[tokio::test]
    async fn test_grpc_unary_call() {
        let (path, _dir) = make_socket_path("unary");
        let _server = spawn_grpc_server(path.clone()).await;
        let mut client = create_client_with_retry(&path).await;

        let request = Request::new(EchoRequest {
            message: "Hello, IPC!".to_string(),
        });

        let response = timeout(Duration::from_secs(2), client.unary_echo(request))
            .await
            .expect("timeout")
            .expect("rpc failed");

        assert_eq!(response.into_inner().message, "Hello, IPC!");
    }

    #[tokio::test]
    async fn test_grpc_bidi_streaming() {
        let (path, _dir) = make_socket_path("bidi");
        let _server = spawn_grpc_server(path.clone()).await;
        let mut client = create_client_with_retry(&path).await;

        let (tx, rx) = mpsc::channel(4);
        let request_stream = ReceiverStream::new(rx);

        let response = client
            .bidi_echo(Request::new(request_stream))
            .await
            .expect("rpc failed");

        let mut response_stream = response.into_inner();

        for i in 1..=3 {
            let msg = format!("Message {}", i);
            tx.send(EchoRequest {
                message: msg.clone(),
            })
            .await
            .unwrap();

            let res = response_stream.message().await.unwrap().unwrap();
            assert_eq!(res.message, msg);
        }
    }

    #[tokio::test]
    async fn test_grpc_large_payload() {
        let (path, _dir) = make_socket_path("large_payload");
        let _server = spawn_grpc_server(path.clone()).await;

        let mut client = create_client_with_retry(&path).await;

        let huge_message = "A".repeat(1024 * 1024);
        let request = Request::new(EchoRequest {
            message: huge_message.clone(),
        });

        let response = timeout(Duration::from_secs(5), client.unary_echo(request))
            .await
            .expect("timeout")
            .expect("rpc failed");

        let response_msg = response.into_inner().message;
        assert_eq!(response_msg.len(), huge_message.len());
        assert_eq!(response_msg, huge_message);
    }

    #[tokio::test]
    async fn test_grpc_multiplexing_concurrent_requests() {
        let (path, _dir) = make_socket_path("multiplexing");
        let _server = spawn_grpc_server(path.clone()).await;

        let client = create_client_with_retry(&path).await;
        let mut handles = vec![];

        const N: usize = 100;

        for i in 0..N {
            let mut c = client.clone();
            handles.push(tokio::spawn(async move {
                let msg = format!("Concurrent request {}", i);
                let request = Request::new(EchoRequest {
                    message: msg.clone(),
                });
                let res = c.unary_echo(request).await.unwrap();
                assert_eq!(res.into_inner().message, msg);
            }));
        }

        for h in handles {
            timeout(Duration::from_secs(5), h)
                .await
                .expect("timeout")
                .expect("task panicked");
        }
    }
}
