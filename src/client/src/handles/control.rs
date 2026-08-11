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
    use knot_proto::commands::v1::{StatusResponse, SyncResponse, SyncResult};
    use knot_proto::data::v1::WorkspaceManifest;
    use tonic::Code;
    use tonic::Response;
    use tonic::metadata::MetadataValue;

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

        let status = controller.status().await.unwrap();
        assert!(status.services.is_empty());
    }

    #[tokio::test]
    async fn status_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.status_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                Err(tonic::Status::permission_denied("forbidden"))
            }));
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
        use tokio_stream::StreamExt;

        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                let metadata = request
                    .metadata
                    .expect("sync request must contain workspace metadata");

                assert_eq!(metadata.workspace_id, "test_id");
                assert_eq!(metadata.root_path, "/test/path");

                assert!(
                    request.manifest.is_some(),
                    "sync request must contain workspace manifest"
                );

                let (tx, rx) = tokio::sync::mpsc::channel(1);

                tx.try_send(Ok(SyncResponse {
                    event: Some(knot_proto::commands::v1::sync_response::Event::Result(
                        SyncResult {
                            services_added: vec!["service_a".to_string()],
                            services_removed: vec![],
                            services_changed: vec![],
                        },
                    )),
                }))
                .unwrap();

                let mut response = Response::new(tokio_stream::wrappers::ReceiverStream::new(rx));
                response
                    .metadata_mut()
                    .insert("x-command-id", "cmd_sync_123".parse().unwrap());
                Ok(response)
            }));
        }

        let mut sync_handle = controller.sync(WorkspaceManifest::default()).await.unwrap();

        let response = sync_handle
            .next()
            .await
            .expect("expected SyncResponse event")
            .expect("SyncResponse stream returned an error");

        match response.event {
            Some(knot_proto::commands::v1::sync_response::Event::Result(result)) => {
                assert_eq!(result.services_added.len(), 1);
                assert_eq!(result.services_added[0], "service_a");
            }

            event => {
                panic!("expected SyncResult event, got: {event:?}");
            }
        }
    }

    #[tokio::test]
    async fn sync_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.sync_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                Err(tonic::Status::failed_precondition("workspace locked"))
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
    async fn up_sends_workspace_id_and_returns_command_handle() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;
            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");
                assert!(request.services.is_empty(), "up() must start all services");

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                let mut response = Response::new(tokio_stream::wrappers::ReceiverStream::new(rx));

                response.metadata_mut().insert(
                    "x-command-id",
                    "cmd_up_123".parse::<MetadataValue<_>>().unwrap(),
                );

                Ok(response)
            }));
        }

        let command = controller.up().await.unwrap();

        assert_eq!(command.command_id, "cmd_up_123");
    }

    #[tokio::test]
    async fn up_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
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
    async fn down_sends_workspace_id_and_returns_command_handle() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;
            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, "test_id");
                assert!(request.services.is_empty(), "down() must stop all services");

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                let mut response = Response::new(tokio_stream::wrappers::ReceiverStream::new(rx));

                response.metadata_mut().insert(
                    "x-command-id",
                    "cmd_down_123".parse::<MetadataValue<_>>().unwrap(),
                );

                Ok(response)
            }));
        }

        let command = controller.down().await.unwrap();

        assert_eq!(command.command_id, "cmd_down_123");
    }

    #[tokio::test]
    async fn down_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.down_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
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

        let status = controller.status().await.unwrap();

        assert!(status.services.is_empty());
    }

    #[tokio::test]
    async fn command_methods_use_empty_service_list_for_all_services() {
        let (mock, client) = spawn_mock_server().await;
        let controller = control_handle(client);

        {
            let mut handler = mock.up_handler.lock().await;
            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                let mut response = Response::new(tokio_stream::wrappers::ReceiverStream::new(rx));

                response.metadata_mut().insert(
                    "x-command-id",
                    "up-test".parse::<MetadataValue<_>>().unwrap(),
                );

                Ok(response)
            }));
        }

        {
            let mut handler = mock.down_handler.lock().await;
            *handler = Some(Box::new(|req| {
                assert!(req.into_inner().services.is_empty());

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                let mut response = Response::new(tokio_stream::wrappers::ReceiverStream::new(rx));

                response.metadata_mut().insert(
                    "x-command-id",
                    "down-test".parse::<MetadataValue<_>>().unwrap(),
                );

                Ok(response)
            }));
        }

        let up = controller.up().await.unwrap();
        let down = controller.down().await.unwrap();

        assert_eq!(up.command_id, "up-test");
        assert_eq!(down.command_id, "down-test");
    }
}
