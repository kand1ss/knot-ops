use crate::errors::ClientError;
use crate::handles::ExecutionHandle;
use crate::policies::PolicyConfig;
use crate::utils::request;
use knot_proto::v1::commands::{SyncRequest, SyncResponse};
use knot_proto::v1::{
    commands::{
        CommitRequest, CommitResponse, DownRequest, DownResponse, StatusRequest, StatusResponse,
        UpRequest, UpResponse,
    },
    config::WorkspaceManifest,
    daemon_service_client::DaemonServiceClient,
};
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
    pub(crate) workspace_id: String,
    pub(crate) client: DaemonServiceClient<Channel>,
    pub(crate) policy: Arc<PolicyConfig>,
}

impl ControlHandle {
    /// Extracts the `x-execution-id` from the gRPC response metadata.
    fn get_execution_id<R>(response: &Response<R>) -> Result<String, ClientError> {
        response
            .metadata()
            .get("x-execution-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| {
                let err_msg = "daemon did not return an 'x-execution-id' header";
                error!(err_msg);
                ClientError::Contract(err_msg.to_string())
            })
    }

    /// Commits the local workspace configuration to the daemon.
    ///
    /// # Arguments
    ///
    /// * `workspace_manifest` - The `Workspace` configuration to commit.
    ///
    /// # Returns
    ///
    /// Returns a `ExecutionHandle<CommitResponse>` tied to the specific execution.
    #[instrument(
        skip(self, workspace_manifest),
        name = "commit_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn commit(
        &self,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<CommitResponse, ClientError> {
        debug!("sending 'commit' request to daemon");

        let mut client = self.client.clone();
        let response = client
            .commit(request(
                CommitRequest {
                    workspace_id: self.workspace_id.clone(),
                    manifest: Some(workspace_manifest),
                },
                Some(self.policy.timeout.fast_commands),
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to commit workspace configuration");
                e
            })?;
        Ok(response.into_inner())
    }

    /// Starts all services managed by the daemon.
    ///
    /// This method initiates the startup sequence and returns a server-stream
    /// to monitor the execution progress (e.g., tasks starting, completing, or failing).
    ///
    /// # Returns
    ///
    /// Returns a `ExecutionHandle<UpResponse>` tied to the specific execution.
    #[instrument(
        skip(self),
        name = "up_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn up(
        &self,
        services: &[String],
    ) -> Result<ExecutionHandle<UpResponse>, ClientError> {
        debug!(services_count = services.len(), "initiating 'up' execution");

        let mut client = self.client.clone();
        let response = client
            .up(request(
                UpRequest {
                    services: Vec::from(services),
                    workspace_id: self.workspace_id.clone(),
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'up' execution");
                e
            })?;
        let execution_id = Self::get_execution_id(&response)?;
        info!(execution_id = %execution_id, "successfully initiated 'up' execution stream");

        Ok(ExecutionHandle::new(
            execution_id,
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
    /// Returns a `ExecutionHandle<DownResponse>` tied to the specific execution.
    #[instrument(
        skip(self),
        name = "down_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn down(
        &self,
        services: &[String],
    ) -> Result<ExecutionHandle<DownResponse>, ClientError> {
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
        let execution_id = Self::get_execution_id(&response)?;
        info!(execution_id = %execution_id, "successfully initiated 'down' execution stream");

        Ok(ExecutionHandle::new(
            execution_id,
            response.into_inner(),
            client,
        ))
    }

    /// Reconciles running services with the workspace's committed manifest.
    ///
    /// Unlike [`Self::up`] and [`Self::down`], which are imperative — they act
    /// only on the services explicitly named — `sync` is declarative: it takes
    /// no service list and instead drives the entire runtime state toward
    /// whatever was last committed via `Commit`. The daemon computes the full
    /// diff itself (start missing services, stop removed ones, restart changed
    /// ones) and streams the resulting plan and task events exactly like `up`/`down`.
    ///
    /// Call this method when a [`Self::handshake`] (or `Status`) call reports
    /// runtime drift — i.e. the committed manifest no longer matches what is
    /// actually running. Committing a new manifest does **not** apply it to
    /// the runtime by itself; `sync` is the only operation that closes that
    /// gap. Until `sync` runs, the workspace is manifest-in-sync but
    /// runtime-drifted — a normal, expected intermediate state, not an error.
    ///
    /// Services stopped explicitly via [`Self::down`] are treated as an
    /// override and are skipped by `sync` until either the service is started
    /// again via [`Self::up`] or a new manifest is committed — `sync` will not
    /// silently resurrect a service you just told the daemon to stop.
    ///
    /// # Returns
    ///
    /// Returns an `ExecutionHandle<SyncResponse>` tied to the specific
    /// execution, streaming the same `CommandPlan`/`TaskStarting`/`TaskFailed`/
    /// `TaskSkipped` event vocabulary as `up`/`down`, followed by a
    /// `SyncResult` summarizing services added, removed, and changed.
    #[instrument(
        skip(self),
        name = "sync_command",
        fields(workspace_id = %self.workspace_id)
    )]
    pub async fn sync(&self) -> Result<ExecutionHandle<SyncResponse>, ClientError> {
        debug!(workspace_id = %self.workspace_id, "initiating 'sync' execution");

        let mut client = self.client.clone();
        let response = client
            .sync(request(
                SyncRequest {
                    workspace_id: self.workspace_id.clone(),
                },
                self.policy.timeout.long_streams,
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to initiate 'sync' execution");
                e
            })?;

        let execution_id = Self::get_execution_id(&response)?;
        info!(execution_id = %execution_id, "successfully initiated 'sync' execution stream");

        Ok(ExecutionHandle::new(
            execution_id,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::{control_handle, spawn_mock_server};

    use knot_proto::v1::{
        commands::{CommitResponse, StatusResponse},
        config::WorkspaceManifest,
    };

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
        command_id: &str,
        responses: impl IntoIterator<Item = Result<T, Status>>,
    ) -> Response<tokio_stream::wrappers::ReceiverStream<Result<T, Status>>> {
        let mut response = command_stream(responses);

        response.metadata_mut().insert(
            "x-execution-id",
            command_id
                .parse::<MetadataValue<_>>()
                .expect("invalid test execution id"),
        );

        response
    }

    fn command_with_id<T>(execution_id: &str, response: T) -> Response<T> {
        let mut res = Response::new(response);
        res.metadata_mut().insert(
            "x-execution-id",
            execution_id
                .parse::<MetadataValue<_>>()
                .expect("invalid test execution id"),
        );
        res
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
    async fn commit_sends_workspace_id_and_manifest() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        let expected_manifest = WorkspaceManifest::default();

        {
            let mut handler = mock.commit_handler.lock().await;

            let expected_manifest = expected_manifest.clone();

            *handler = Some(Box::new(move |req| {
                let request = req.into_inner();

                let metadata = request.workspace_id;

                assert_eq!(metadata, "test_id");
                let manifest = request
                    .manifest
                    .expect("sync request must contain workspace manifest");

                assert_eq!(manifest, expected_manifest);

                Ok(command_with_id(
                    "cmd_sync_123",
                    CommitResponse {
                        services_added: vec!["service_a".to_string()],
                        services_removed: vec![],
                        services_changed: vec![],
                    },
                ))
            }));
        }

        let response = controller
            .commit(expected_manifest)
            .await
            .expect("sync request should succeed");

        assert_eq!(response.services_added, vec!["service_a"]);
        assert!(response.services_removed.is_empty());
        assert!(response.services_changed.is_empty());
    }

    #[tokio::test]
    async fn commit_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::failed_precondition("workspace locked"))
            }));
        }

        let result = controller.commit(WorkspaceManifest::default()).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::FailedPrecondition
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

        let mut command = controller.up(&[]).await.expect("up request should succeed");

        assert_eq!(command.execution_id, "cmd_up_123");

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
                Ok(command_stream([Ok(UpResponse::default())]))
            }));
        }

        let result = controller.up(&[]).await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-execution-id")
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

        let result = controller.up(&[]).await;

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

        let mut command = controller.up(&[]).await.expect("up request should succeed");

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
                    [Ok(DownResponse::default())],
                ))
            }));
        }

        let mut command = controller
            .down(&[])
            .await
            .expect("down request should succeed");

        assert_eq!(command.execution_id, "cmd_down_123");

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
    async fn sync_sends_workspace_id_and_all_services_marker() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");
                Ok(command_stream_with_id(
                    "cmd_down_123",
                    [Ok(SyncResponse::default())],
                ))
            }));
        }

        let mut command = controller
            .sync()
            .await
            .expect("down request should succeed");

        assert_eq!(command.execution_id, "cmd_down_123");

        assert!(command.next().await.unwrap().is_ok());

        assert!(command.next().await.is_none());
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

        let result = controller.sync().await;

        assert!(matches!(result, Err(ClientError::Contract(_))));
    }

    #[tokio::test]
    async fn sync_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let result = controller.sync().await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
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
                    "down-error",
                    [Err(Status::internal("service shutdown failed"))],
                ))
            }));
        }

        let mut command = controller
            .sync()
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

        let up = controller.up(&[]).await.expect("up should succeed");

        let down = controller.down(&[]).await.expect("down should succeed");

        assert_eq!(up.execution_id, "up-test");

        assert_eq!(down.execution_id, "down-test");
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

        let command = controller.up(&[]).await.expect("up should succeed");

        assert_eq!(command.execution_id, "server-generated-id");
    }
}
