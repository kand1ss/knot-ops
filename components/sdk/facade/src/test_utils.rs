use crate::handles::ControlHandle;
use crate::policies::PolicyConfig;
use async_trait::async_trait;
use knot_proto::v1::commands::{SyncRequest, SyncResponse};
use knot_proto::v1::{
    commands::{
        CommitRequest, CommitResponse, DownRequest, DownResponse, HandshakeRequest,
        HandshakeResponse, LogsRequest, LogsResponse, StatusRequest, StatusResponse, UpRequest,
        UpResponse,
    },
    daemon_service_client::DaemonServiceClient,
    daemon_service_server::{DaemonService, DaemonServiceServer},
    execution::{CancelExecutionRequest, CancelExecutionResponse},
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};

type BoxHandler<Req, Res> =
    Box<dyn FnMut(Request<Req>) -> Result<Response<Res>, Status> + Send + Sync>;

type Handler<Req, Res> = Arc<Mutex<Option<BoxHandler<Req, Res>>>>;

type StreamHandler<Req, Event> = Arc<
    Mutex<
        Option<
            Box<
                dyn FnMut(
                        Request<Req>,
                    )
                        -> Result<Response<ReceiverStream<Result<Event, Status>>>, Status>
                    + Send
                    + Sync,
            >,
        >,
    >,
>;

async fn call_handler<Req, Res>(
    handler: &Handler<Req, Res>,
    request: Request<Req>,
    name: &str,
) -> Result<Response<Res>, Status> {
    match handler.lock().await.as_mut() {
        Some(h) => h(request),
        None => Err(Status::unimplemented(format!("{name} not mocked"))),
    }
}

#[derive(Default, Clone)]
pub struct MockKnotDaemon {
    pub handshake_handler: Handler<HandshakeRequest, HandshakeResponse>,
    pub commit_handler: Handler<CommitRequest, CommitResponse>,
    pub status_handler: Handler<StatusRequest, StatusResponse>,
    pub cancel_execution_handler: Handler<CancelExecutionRequest, CancelExecutionResponse>,
    pub up_handler: StreamHandler<UpRequest, UpResponse>,
    pub down_handler: StreamHandler<DownRequest, DownResponse>,
    pub sync_handler: StreamHandler<SyncRequest, SyncResponse>,
}

#[async_trait]
impl DaemonService for MockKnotDaemon {
    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        call_handler(&self.handshake_handler, request, "handshake").await
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        call_handler(&self.commit_handler, request, "sync").await
    }

    type UpStream = ReceiverStream<Result<UpResponse, Status>>;

    async fn up(&self, request: Request<UpRequest>) -> Result<Response<Self::UpStream>, Status> {
        match self.up_handler.lock().await.as_mut() {
            Some(h) => h(request),
            None => Err(Status::unimplemented("up not mocked")),
        }
    }

    type DownStream = ReceiverStream<Result<DownResponse, Status>>;
    async fn down(
        &self,
        request: Request<DownRequest>,
    ) -> Result<Response<Self::DownStream>, Status> {
        match self.down_handler.lock().await.as_mut() {
            Some(h) => h(request),
            None => Err(Status::unimplemented("down not mocked")),
        }
    }

    type SyncStream = ReceiverStream<Result<SyncResponse, Status>>;
    async fn sync(
        &self,
        request: Request<SyncRequest>,
    ) -> Result<Response<Self::SyncStream>, Status> {
        match self.sync_handler.lock().await.as_mut() {
            Some(h) => h(request),
            None => Err(Status::unimplemented("sync not mocked")),
        }
    }

    async fn status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        call_handler(&self.status_handler, request, "status").await
    }
    type LogsStream = ReceiverStream<Result<LogsResponse, Status>>;

    async fn logs(
        &self,
        _request: Request<LogsRequest>,
    ) -> Result<Response<Self::LogsStream>, Status> {
        Err(Status::unimplemented("logs not mocked"))
    }
    async fn cancel_execution(
        &self,
        request: Request<CancelExecutionRequest>,
    ) -> Result<Response<CancelExecutionResponse>, Status> {
        call_handler(&self.cancel_execution_handler, request, "cancel_command").await
    }
}

pub async fn spawn_mock_server() -> (MockKnotDaemon, DaemonServiceClient<Channel>) {
    let mock = MockKnotDaemon::default();
    let service = DaemonServiceServer::new(mock.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        Server::builder()
            .add_service(service)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    (mock, DaemonServiceClient::new(channel))
}

pub fn control_handle(client: DaemonServiceClient<Channel>) -> ControlHandle {
    ControlHandle {
        workspace_id: "test_id".to_string(),
        client,
        policy: Arc::new(PolicyConfig::default()),
    }
}
