use std::{sync::Arc, time::Duration};

use knot_proto::v1::{
    commands::{
        CommitRequest, CommitResponse, DownRequest, DownResponse, HandshakeRequest,
        HandshakeResponse, StatusRequest, StatusResponse, UpRequest, UpResponse,
    },
    daemon_service_client::DaemonServiceClient,
};
use tonic::{Request, Response, Status, Streaming, transport::Channel};
use tracing::{error, warn};

use crate::{errors::ClientError, handles::TaskHandle, policies::PolicyConfig};

pub(crate) const METADATA_TASK_ID_HEADER: &str = "x-task-id";

#[derive(Debug, Clone)]
pub(crate) struct GuardedDaemonClient {
    inner: DaemonServiceClient<Channel>,
    policy: Arc<PolicyConfig>,
}

impl GuardedDaemonClient {
    pub fn new(inner: DaemonServiceClient<Channel>, policy: Arc<PolicyConfig>) -> Self {
        Self { inner, policy }
    }

    pub fn request<R>(req: R, timeout: Option<Duration>) -> Request<R> {
        let mut request = Request::new(req);
        if let Some(duration) = timeout {
            request.set_timeout(duration);
        }
        request
    }

    async fn call_rpc<T, Fut>(&self, fut: Fut) -> Result<Response<T>, ClientError>
    where
        Fut: std::future::Future<Output = Result<Response<T>, Status>>,
    {
        fut.await.map_err(|status| {
            if status.code() == tonic::Code::Aborted {
                warn!("daemon has rejected a provided revision; request aborted");
                ClientError::WrongRevision
            } else {
                ClientError::Protocol(status)
            }
        })
    }

    async fn execute_task_cmd<T, Fut>(
        &self,
        fut: Fut,
    ) -> Result<(String, Streaming<T>), ClientError>
    where
        Fut: std::future::Future<Output = Result<Response<Streaming<T>>, Status>>,
    {
        let response = self.call_rpc(fut).await?;

        let task_id = response
            .metadata()
            .get(METADATA_TASK_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| {
                let err_msg =
                    format!("daemon did not return an '{METADATA_TASK_ID_HEADER}' header");
                error!(err_msg);
                ClientError::Contract(err_msg.to_string())
            })?;

        Ok((task_id, response.into_inner()))
    }

    pub async fn handshake(&self, req: HandshakeRequest) -> Result<HandshakeResponse, ClientError> {
        let grpc_req = Self::request(req, self.policy.timeout.long_streams);
        let mut client = self.inner.clone();
        let response = self.call_rpc(client.handshake(grpc_req)).await?;
        Ok(response.into_inner())
    }

    pub async fn commit(&self, req: CommitRequest) -> Result<CommitResponse, ClientError> {
        let grpc_req = Self::request(req, self.policy.timeout.long_streams);
        let mut client = self.inner.clone();
        let response = self.call_rpc(client.commit(grpc_req)).await?;
        Ok(response.into_inner())
    }

    pub async fn up(&self, req: UpRequest) -> Result<TaskHandle<UpResponse>, ClientError> {
        let grpc_req = Self::request(req, self.policy.timeout.long_streams);
        let mut client = self.inner.clone();
        let (task_id, stream) = self.execute_task_cmd(client.up(grpc_req)).await?;
        Ok(TaskHandle::new(task_id, stream, client))
    }

    pub async fn down(&self, req: DownRequest) -> Result<TaskHandle<DownResponse>, ClientError> {
        let grpc_req = Self::request(req, self.policy.timeout.long_streams);
        let mut client = self.inner.clone();
        let (task_id, stream) = self.execute_task_cmd(client.down(grpc_req)).await?;
        Ok(TaskHandle::new(task_id, stream, client.clone()))
    }

    pub async fn status(&self, req: StatusRequest) -> Result<StatusResponse, ClientError> {
        let grpc_req = Self::request(req, Some(self.policy.timeout.fast_commands));
        let mut client = self.inner.clone();

        let response = self.call_rpc(client.status(grpc_req)).await.map_err(|e| {
            error!(error = %e, "failed to fetch status from daemon");
            e
        })?;

        Ok(response.into_inner())
    }
}
