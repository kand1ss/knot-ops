use crate::handles::ControlHandle;
use crate::policies::PolicyConfig;
use async_trait::async_trait;
use knot_proto::data::v1::WorkspaceMetadata;
use knot_proto::{
    api::v1::{
        daemon_service_client::DaemonServiceClient,
        daemon_service_server::{DaemonService, DaemonServiceServer},
    },
    command::v1::{CancelCommandRequest, CancelCommandResponse},
    commands::v1::{DownRequest,
                   DownResponse, HandshakeRequest, HandshakeResponse,
                   LogsRequest, LogsResponse, StatusRequest, StatusResponse, SyncRequest, SyncResponse, UpRequest,
                   UpResponse,
    },
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
    pub sync_handler: StreamHandler<SyncRequest, SyncResponse>,
    pub status_handler: Handler<StatusRequest, StatusResponse>,
    pub cancel_command_handler: Handler<CancelCommandRequest, CancelCommandResponse>,
    pub up_handler: StreamHandler<UpRequest, UpResponse>,
    pub down_handler: StreamHandler<DownRequest, DownResponse>,
}

#[async_trait]
impl DaemonService for MockKnotDaemon {
    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        call_handler(&self.handshake_handler, request, "handshake").await
    }

    type SyncStream = ReceiverStream<Result<SyncResponse, Status>>;
    async fn sync(&self, request: Request<SyncRequest>) -> Result<Response<Self::SyncStream>, Status> {
        call_handler(&self.sync_handler, request, "sync").await
    }

    async fn status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        call_handler(&self.status_handler, request, "status").await
    }

    async fn cancel_command(
        &self,
        request: Request<CancelCommandRequest>,
    ) -> Result<Response<CancelCommandResponse>, Status> {
        call_handler(&self.cancel_command_handler, request, "cancel_command").await
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

    type LogsStream = ReceiverStream<Result<LogsResponse, Status>>;
    async fn logs(
        &self,
        _request: Request<LogsRequest>,
    ) -> Result<Response<Self::LogsStream>, Status> {
        Err(Status::unimplemented("logs not mocked"))
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
        workspace_meta: WorkspaceMetadata {
            workspace_id: "test_id".to_string(),
            root_path: "/test/path".to_string(),
        },
        client,
        policy: Arc::new(PolicyConfig::default()),
    }
}