use crate::errors::ClientError;
use crate::handles::CommandHandle;
use crate::policies::PolicyConfig;
use crate::utils::request;
use knot_proto::api::v1::daemon_service_client::DaemonServiceClient;
use knot_proto::commands::v1::{
    DownRequest, DownResponse, StatusRequest, StatusResponse, SyncRequest, SyncResponse, UpRequest,
    UpResponse,
};
use knot_proto::data::v1::{WorkspaceManifest, WorkspaceMetadata};
use std::sync::Arc;
use tonic::{Response, transport::Channel};
use tracing::{debug, error, info, instrument};

/// A handle for controlling and communicating with the Knot daemon.
///
/// `ControllerHandle` wraps a gRPC client communicating over an IPC socket.
/// It provides methods to synchronize configurations, manage service lifecycles
/// (`up`/`down`), and observe the daemon's state.
#[derive(Debug)]
pub struct ControlHandle {
    pub(crate) workspace_meta: WorkspaceMetadata,
    pub(crate) client: DaemonServiceClient<Channel>,
    pub(crate) policy: Arc<PolicyConfig>,
}

impl ControlHandle {
    /// Extracts the `x-command-id` from the gRPC response metadata.
    fn get_command_id<R>(response: &Response<R>) -> Result<String, ClientError> {
        response
            .metadata()
            .get("x-command-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| {
                let err_msg = "daemon did not return an 'x-command-id' header";
                error!(err_msg);
                ClientError::Contract(err_msg.to_string())
            })
    }

    /// Synchronizes the local workspace configuration with the daemon.
    ///
    /// # Arguments
    ///
    /// * `workspace_manifest` - The `Workspace` configuration to apply.
    ///
    /// # Returns
    ///
    /// Returns a `CommandHandle<SyncResponse>` tied to the specific command execution.
    #[instrument(skip(self, workspace_manifest), name = "sync_command")]
    pub async fn sync(
        &self,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<CommandHandle<SyncResponse>, ClientError> {
        debug!("sending workspace configuration to daemon");

        let mut client = self.client.clone();
        let response = client
            .sync(request(
                SyncRequest {
                    metadata: Some(self.workspace_meta.clone()),
                    manifest: Some(workspace_manifest),
                },
                Some(self.policy.timeout.fast_commands),
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to synchronize workspace configuration");
                e
            })?;
        let command_id = Self::get_command_id(&response)?;
        info!(command_id = %command_id, "successfully initiated 'sync' command stream");

        Ok(CommandHandle::new(
            command_id,
            response.into_inner(),
            client,
        ))
    }

    /// Starts all services managed by the daemon.
    ///
    /// This method initiates the startup sequence and returns a server-stream
    /// to monitor the execution progress (e.g., tasks starting, completing, or failing).
    ///
    /// # Returns
    ///
    /// Returns a `CommandHandle<UpResponse>` tied to the specific command execution.
    #[instrument(skip(self), name = "up_command")]
    pub async fn up(&self) -> Result<CommandHandle<UpResponse>, ClientError> {
        debug!("initiating 'up' command for all services");

        let mut client = self.client.clone();
        let response = client
            .up(request(
                UpRequest {
                    services: vec![],
                    workspace_id: self.workspace_meta.workspace_id.clone(),
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'up' command");
                e
            })?;
        let command_id = Self::get_command_id(&response)?;
        info!(command_id = %command_id, "successfully initiated 'up' command stream");

        Ok(CommandHandle::new(
            command_id,
            response.into_inner(),
            client,
        ))
    }

    /// Stops all services managed by the daemon.
    ///
    /// This method sends a `Down` request to gracefully terminate all
    /// active services. Like `Self::up`, it provides a stream to monitor
    /// the shutdown sequence.
    ///
    /// # Returns
    ///
    /// Returns a `CommandHandle<DownResponse>` tied to the specific command execution.
    #[instrument(skip(self), name = "down_command")]
    pub async fn down(&self) -> Result<CommandHandle<DownResponse>, ClientError> {
        debug!("initiating 'down' command for all services");

        let mut client = self.client.clone();
        let response = client
            .down(request(
                DownRequest {
                    services: vec![],
                    workspace_id: self.workspace_meta.workspace_id.clone(),
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'down' command");
                e
            })?;
        let command_id = Self::get_command_id(&response)?;
        info!(command_id = %command_id, "successfully initiated 'down' command stream");

        Ok(CommandHandle::new(
            command_id,
            response.into_inner(),
            client,
        ))
    }

    /// Fetches the current status of all managed services in the workspace.
    ///
    /// Unlike `up` or `down`, this is a request-response operation that
    /// returns the immediate state of the workspace without opening a long-running stream.
    ///
    /// # Returns
    ///
    /// Returns a `StatusResponse` containing details for each service,
    /// such as PID, uptime, and health status.
    #[instrument(skip(self), name = "status_command")]
    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        debug!("fetching daemon status");

        let mut client = self.client.clone();
        let response = client
            .status(request(
                StatusRequest {
                    services: vec![],
                    workspace_id: self.workspace_meta.workspace_id.clone(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::{control_handle, spawn_mock_server};

    use knot_proto::{
        commands::v1::{StatusResponse, SyncResponse, SyncResult, sync_response},
        data::v1::WorkspaceManifest,
    };

    use tokio_stream::StreamExt;
    use tonic::{Code, Response, Status, metadata::MetadataValue};

    fn command_stream<T>(
        responses: impl IntoIterator<Item = Result<T, Status>>,
    ) -> tonic::Response<tokio_stream::wrappers::ReceiverStream<Result<T, Status>>> {
        let responses = responses.into_iter().collect::<Vec<_>>();

        let (tx, rx) = tokio::sync::mpsc::channel(responses.len().max(1));

        for response in responses {
            tx.try_send(response)
                .expect("failed to populate mock response stream");
        }

        Response::new(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    fn command_stream_with_id<T>(
        command_id: &str,
        responses: impl IntoIterator<Item = Result<T, Status>>,
    ) -> Response<tokio_stream::wrappers::ReceiverStream<Result<T, Status>>> {
        let mut response = command_stream(responses);

        response.metadata_mut().insert(
            "x-command-id",
            command_id
                .parse::<MetadataValue<_>>()
                .expect("invalid test command id"),
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
    async fn sync_sends_workspace_metadata_and_manifest() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        let expected_manifest = WorkspaceManifest::default();

        {
            let mut handler = mock.sync_handler.lock().await;

            let expected_manifest = expected_manifest.clone();

            *handler = Some(Box::new(move |req| {
                let request = req.into_inner();

                let metadata = request
                    .metadata
                    .expect("sync request must contain workspace metadata");

                assert_eq!(metadata.workspace_id, "test_id");

                assert_eq!(metadata.root_path, "/test/path");

                let manifest = request
                    .manifest
                    .expect("sync request must contain workspace manifest");

                assert_eq!(manifest, expected_manifest);

                Ok(command_stream_with_id(
                    "cmd_sync_123",
                    [Ok(SyncResponse {
                        event: Some(sync_response::Event::Result(SyncResult {
                            services_added: vec!["service_a".to_string()],
                            services_removed: vec![],
                            services_changed: vec![],
                        })),
                    })],
                ))
            }));
        }

        let mut handle = controller
            .sync(expected_manifest)
            .await
            .expect("sync request should succeed");

        assert_eq!(handle.command_id, "cmd_sync_123");

        let response = handle
            .next()
            .await
            .expect("expected SyncResponse event")
            .expect("SyncResponse stream returned an error");

        match response.event {
            Some(sync_response::Event::Result(result)) => {
                assert_eq!(result.services_added, vec!["service_a"]);

                assert!(result.services_removed.is_empty());

                assert!(result.services_changed.is_empty());
            }

            event => {
                panic!("expected SyncResult event, got: {event:?}");
            }
        }

        assert!(
            handle.next().await.is_none(),
            "sync stream should terminate"
        );
    }

    #[tokio::test]
    async fn sync_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::failed_precondition("workspace locked"))
            }));
        }

        let result = controller.sync(WorkspaceManifest::default()).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::FailedPrecondition
        ),);
    }

    #[tokio::test]
    async fn sync_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream([Ok(SyncResponse::default())]))
            }));
        }

        let result = controller.sync(WorkspaceManifest::default()).await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-command-id")
        ),);
    }

    #[tokio::test]
    async fn sync_propagates_stream_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream_with_id(
                    "sync-error",
                    [
                        Ok(SyncResponse::default()),
                        Err(Status::internal("sync execution failed")),
                    ],
                ))
            }));
        }

        let mut handle = controller
            .sync(WorkspaceManifest::default())
            .await
            .expect("sync request should succeed");

        assert!(handle.next().await.unwrap().is_ok());

        let result = handle.next().await.expect("stream error must be present");

        match result {
            Err(status) => {
                assert_eq!(status.code(), Code::Internal);

                assert_eq!(status.message(), "sync execution failed");
            }

            Ok(_) => {
                panic!("expected stream error");
            }
        }
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
                    [Ok(knot_proto::commands::v1::UpResponse::default())],
                ))
            }));
        }

        let mut command = controller.up().await.expect("up request should succeed");

        assert_eq!(command.command_id, "cmd_up_123");

        assert!(command.next().await.unwrap().is_ok());

        assert!(command.next().await.is_none());
    }

    #[tokio::test]
    async fn up_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(command_stream([Ok(
                    knot_proto::commands::v1::UpResponse::default(),
                )]))
            }));
        }

        let result = controller.up().await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-command-id")
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

        let result = controller.up().await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
        ),);
    }

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

        let mut command = controller.up().await.expect("up request should succeed");

        let result = command.next().await.expect("stream error must exist");

        match result {
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
                    [Ok(knot_proto::commands::v1::DownResponse::default())],
                ))
            }));
        }

        let mut command = controller
            .down()
            .await
            .expect("down request should succeed");

        assert_eq!(command.command_id, "cmd_down_123");

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
                Ok(command_stream([Ok(
                    knot_proto::commands::v1::DownResponse::default(),
                )]))
            }));
        }

        let result = controller.down().await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-command-id")
        ),);
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

        let result = controller.down().await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
        ),);
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
            .down()
            .await
            .expect("down request should succeed");

        let result = command.next().await.expect("stream error must exist");

        match result {
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
    async fn command_methods_use_empty_service_list_for_all_services() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                Ok(command_stream_with_id(
                    "up-test",
                    [Ok(knot_proto::commands::v1::UpResponse::default())],
                ))
            }));
        }

        {
            let mut handler = mock.down_handler.lock().await;

            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                Ok(command_stream_with_id(
                    "down-test",
                    [Ok(knot_proto::commands::v1::DownResponse::default())],
                ))
            }));
        }

        let up = controller.up().await.expect("up should succeed");

        let down = controller.down().await.expect("down should succeed");

        assert_eq!(up.command_id, "up-test");

        assert_eq!(down.command_id, "down-test");
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

        let command = controller.up().await.expect("up should succeed");

        assert_eq!(command.command_id, "server-generated-id");
    }
}
