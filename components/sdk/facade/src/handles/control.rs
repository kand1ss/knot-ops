use crate::errors::ClientError;
use crate::handles::{ConnectedHandle, TaskHandle};
use crate::policies::PolicyConfig;
use crate::states::DaemonSession;
use crate::utils::request;
use knot_proto::v1::{
    commands::{DownRequest, DownResponse, StatusRequest, StatusResponse, UpRequest, UpResponse},
    config::WorkspaceManifest,
    daemon_service_client::DaemonServiceClient,
};
use std::sync::Arc;
use tonic::{Response, transport::Channel};
use tracing::{debug, error, info, instrument};

/// A handle for controlling and communicating with the Knot daemon.
///
/// `ControlHandle` wraps a gRPC client communicating over an IPC socket.
/// It provides methods to synchronize configurations, manage services lifecycle
/// (`up`/`down`), and observe the daemon's state.
#[derive(Debug)]
pub struct ControlHandle {
    pub(crate) workspace_id: String,
    pub(crate) expected_revision: String,
    pub(crate) client: DaemonServiceClient<Channel>,
    pub(crate) policy: Arc<PolicyConfig>,
}

// TODO - add aborted request handling when expected revision doesn't match with an actual daemon revision
impl ControlHandle {
    /// Extracts the `x-task-id` from the gRPC response metadata.
    fn get_task_id<R>(response: &Response<R>) -> Result<String, ClientError> {
        response
            .metadata()
            .get("x-task-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| {
                let err_msg = "daemon did not return an 'x-task-id' header";
                error!(err_msg);
                ClientError::Contract(err_msg.to_string())
            })
    }

    pub(crate) fn set_revision(&mut self, revision: String) {
        self.expected_revision = revision
    }

    /// Starts or restarts services managed by the daemon.
    ///
    /// This method initiates the startup sequence and returns a server-stream
    /// to monitor the execution progress (e.g., service starting, running, or failing).
    ///
    /// # Arguments
    ///
    /// * `services` - Services to start or restart
    /// * `prune` - Flag to remove orphaned services in workspace
    ///
    /// # Returns
    ///
    /// Returns a `TaskHandle<UpResponse>` tied to the specific execution.
    #[instrument(
        skip(self),
        name = "up_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn up(
        &self,
        services: &[String],
        prune: bool,
    ) -> Result<TaskHandle<UpResponse>, ClientError> {
        debug!(services_count = services.len(), "initiating 'up' execution");

        let mut client = self.client.clone();
        let response = client
            .up(request(
                UpRequest {
                    services: Vec::from(services),
                    expected_revision: self.expected_revision.clone(),
                    workspace_id: self.workspace_id.clone(),
                    prune,
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'up' execution");
                e
            })?;
        let task_id = Self::get_task_id(&response)?;
        info!(task_id = %task_id, "successfully initiated 'up' execution stream");

        Ok(TaskHandle::new(task_id, response.into_inner(), client))
    }

    /// Stops services managed by the daemon.
    ///
    /// This method sends a `Down` request to gracefully terminate all
    /// active services. Like `Self::up`, it provides a stream to monitor
    /// the shutdown sequence.
    ///
    /// # Arguments
    ///
    /// * `services` - Services to stop
    ///
    /// # Returns
    ///
    /// Returns a `TaskHandle<DownResponse>` tied to the specific execution.
    #[instrument(
        skip(self),
        name = "down_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn down(&self, services: &[String]) -> Result<TaskHandle<DownResponse>, ClientError> {
        debug!(
            services_count = services.len(),
            "initiating 'down' execution"
        );

        let mut client = self.client.clone();
        let response = client
            .down(request(
                DownRequest {
                    services: Vec::from(services),
                    workspace_id: self.workspace_id.clone(),
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'down' execution");
                e
            })?;
        let task_id = Self::get_task_id(&response)?;
        info!(task_id = %task_id, "successfully initiated 'down' execution stream");

        Ok(TaskHandle::new(task_id, response.into_inner(), client))
    }

    /// Fetches the current status of all managed services in the workspace.
    ///
    /// Unlike `up` or `down`, this is a request-response operation that
    /// returns the immediate state of the workspace without opening a long-running stream.
    ///
    /// # Returns
    ///
    /// Returns a `StatusResponse` containing details for each service,
    /// such as uptime, and health status.
    #[instrument(
        skip(self),
        name = "status_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        debug!("fetching daemon status");

        let mut client = self.client.clone();
        let response = client
            .status(request(
                StatusRequest {
                    services: vec![],
                    workspace_id: self.workspace_id.clone(),
                },
                Some(self.policy.timeout.fast_commands),
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to fetch status from daemon");
                e
            })?;

        let res = response.into_inner();
        debug!(
            services_count = res.services.len(),
            "successfully retrieved status"
        );
        Ok(res)
    }

    /// Re-evaluates the workspace against the daemon.
    ///
    /// Consumes this handle: after a configuration change, or after the daemon
    /// rejected a request because of a revision mismatch, the revision held here
    /// is stale. Returns a fresh session in the state the daemon reports.
    pub async fn recheck(self, manifest: WorkspaceManifest) -> Result<DaemonSession, ClientError> {
        ConnectedHandle::from_parts(self.client, self.policy)
            .handshake(self.workspace_id, manifest)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::{control_handle, spawn_mock_server};

    use knot_proto::v1::commands::StatusResponse;

    use tokio_stream::StreamExt;
    use tonic::{Code, Response, Status, metadata::MetadataValue};

    fn command_stream<T>(
        responses: impl IntoIterator<Item = Result<T, Status>>,
    ) -> Response<tokio_stream::wrappers::ReceiverStream<Result<T, Status>>> {
        let responses = responses.into_iter().collect::<Vec<_>>();

        let (tx, rx) = tokio::sync::mpsc::channel(responses.len().max(1));

        for response in responses {
            tx.try_send(response)
                .expect("failed to populate mock response stream");
        }

        Response::new(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    fn command_stream_with_id<T>(
        task_id: &str,
        responses: impl IntoIterator<Item = Result<T, Status>>,
    ) -> Response<tokio_stream::wrappers::ReceiverStream<Result<T, Status>>> {
        let mut response = command_stream(responses);

        response.metadata_mut().insert(
            "x-task-id",
            task_id
                .parse::<MetadataValue<_>>()
                .expect("invalid test task id"),
        );

        response
    }

    #[tokio::test]
    async fn status_returns_daemon_response() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.status_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(StatusResponse::default()))
            }));
        }

        let status = controller
            .status()
            .await
            .expect("status request should succeed");

        assert!(status.services.is_empty(), "expected empty service list");
    }

    #[tokio::test]
    async fn status_sends_workspace_id() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.status_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");

                assert!(
                    request.services.is_empty(),
                    "status() must query all services"
                );

                Ok(Response::new(StatusResponse::default()))
            }));
        }

        controller
            .status()
            .await
            .expect("status request should succeed");
    }

    #[tokio::test]
    async fn status_returns_response_payload_unchanged() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        let expected = StatusResponse::default();

        {
            let mut handler = mock.status_handler.lock().await;

            let expected = expected.clone();

            *handler = Some(Box::new(move |_req| Ok(Response::new(expected.clone()))));
        }

        let actual = controller
            .status()
            .await
            .expect("status request should succeed");

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn status_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.status_handler.lock().await;

            *handler = Some(Box::new(|_req| Err(Status::permission_denied("forbidden"))));
        }

        let result = controller.status().await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::PermissionDenied
        ),);
    }

    #[tokio::test]
    async fn up_sends_workspace_id_and_all_services_marker() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");

                assert!(request.services.is_empty(), "up() must start all services");

                Ok(command_stream_with_id(
                    "cmd_up_123",
                    [Ok(UpResponse::default())],
                ))
            }));
        }

        let mut command = controller
            .up(&[], false)
            .await
            .expect("up request should succeed");

        assert_eq!(command.task_id, "cmd_up_123");
        assert!(command.next().await.unwrap().is_ok());
        assert!(command.next().await.is_none());
    }

    #[tokio::test]
    async fn up_returns_contract_error_when_task_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream([Ok(UpResponse::default())]))
            }));
        }

        let result = controller.up(&[], false).await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-task-id")
        ),);
    }

    #[tokio::test]
    async fn up_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let result = controller.up(&[], false).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
        ),);
    }

    // TODO - add up test when an provided expected revision doesn't match with actual

    #[tokio::test]
    async fn up_propagates_stream_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream_with_id(
                    "up-error",
                    [Err(Status::internal("service startup failed"))],
                ))
            }));
        }

        let mut command = controller
            .up(&[], false)
            .await
            .expect("up request should succeed");
        match command.next().await.expect("stream error must exist") {
            Err(status) => {
                assert_eq!(status.code(), Code::Internal);

                assert_eq!(status.message(), "service startup failed");
            }

            Ok(_) => {
                panic!("expected stream error");
            }
        }
    }

    #[tokio::test]
    async fn down_sends_workspace_id_and_all_services_marker() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");

                assert!(request.services.is_empty(), "down() must stop all services");

                Ok(command_stream_with_id(
                    "cmd_down_123",
                    [Ok(DownResponse::default())],
                ))
            }));
        }

        let mut command = controller
            .down(&[])
            .await
            .expect("down request should succeed");

        assert_eq!(command.task_id, "cmd_down_123");

        assert!(command.next().await.unwrap().is_ok());

        assert!(command.next().await.is_none());
    }

    #[tokio::test]
    async fn down_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream([Ok(DownResponse::default())]))
            }));
        }

        let result = controller.down(&[]).await;

        assert!(matches!(result, Err(ClientError::Contract(_))));
    }

    #[tokio::test]
    async fn down_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let result = controller.down(&[]).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
        ),);
    }

    fn ensure_stream_error<T>(res: Result<T, Status>) {
        match res {
            Err(status) => {
                assert_eq!(status.code(), Code::Internal);
                assert_eq!(status.message(), "service shutdown failed");
            }

            Ok(_) => {
                panic!("expected stream error");
            }
        }
    }

    #[tokio::test]
    async fn down_propagates_stream_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream_with_id(
                    "down-error",
                    [Err(Status::internal("service shutdown failed"))],
                ))
            }));
        }

        let mut command = controller
            .down(&[])
            .await
            .expect("down request should succeed");

        let result = command.next().await.expect("stream error must exist");
        ensure_stream_error(result);
    }

    #[tokio::test]
    async fn command_methods_use_empty_service_list_for_all_services() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                Ok(command_stream_with_id(
                    "up-test",
                    [Ok(UpResponse::default())],
                ))
            }));
        }

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                Ok(command_stream_with_id(
                    "down-test",
                    [Ok(DownResponse::default())],
                ))
            }));
        }

        let up = controller.up(&[], false).await.expect("up should succeed");
        let down = controller.down(&[]).await.expect("down should succeed");

        assert_eq!(up.task_id, "up-test");
        assert_eq!(down.task_id, "down-test");
    }

    #[tokio::test]
    async fn command_id_header_is_used_as_command_handle_id() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream_with_id("server-generated-id", []))
            }));
        }

        let command = controller.up(&[], false).await.expect("up should succeed");
        assert_eq!(command.task_id, "server-generated-id");
    }
}
