use crate::errors::ClientError;
use crate::handles::{CommandHandle, ControlHandle};
use knot_proto::commands::v1::SyncResponse;
use knot_proto::data::v1::WorkspaceManifest;
use tracing::{debug, error, info, instrument};

/// A stateful handle representing a connected but unconfigured daemon.
///
/// This handle is returned during the handshake phase if the daemon is currently running
/// but has no active workspace configuration loaded in memory. In this state, lifecycle
/// commands (such as `up` or `down`) are invalid. The only permitted operational action is
/// to provide the initial configuration via the [`Self::sync`] method.
#[derive(Debug)]
pub struct UnsyncedHandle {
    pub(crate) controller: ControlHandle,
}

impl UnsyncedHandle {
    /// Pushes the initial workspace configuration to the daemon.
    ///
    /// This method consumes the `UninitializedHandle` to enforce the state machine transition.
    /// Upon a successful synchronization, it returns the underlying connection controller
    /// alongside the daemon's differential response, allowing the caller to upgrade the session
    /// into a fully operational state (e.g., `ReadyHandle`).
    ///
    /// # Arguments
    ///
    /// * `config` - The fully parsed `Workspace` configuration object to be applied.
    ///
    /// # Returns
    ///
    /// Returns a tuple containing the reclaimed `ControllerHandle` and the `SyncResponse`
    /// detailing the applied changes (added, removed, or modified services).
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the gRPC synchronization request fails or if the daemon
    /// rejects the provided configuration.
    #[instrument(skip_all, name = "uninitialized_sync")]
    pub async fn sync(
        self,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<(ControlHandle, CommandHandle<SyncResponse>), ClientError> {
        debug!("pushing initial workspace configuration to uninitialized daemon");

        let response = self
            .controller
            .sync(workspace_manifest)
            .await
            .map_err(|e| {
                error!(error = %e, "failed to synchronize initial workspace configuration");
                e
            })?;

        info!("initial synchronization successful, consuming uninitialized handle");

        // We safely return the underlying controller so the orchestrator can wrap it
        // into the next logical state (like ReadyHandle).
        Ok((self.controller, response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::policies::PolicyConfig;
    use crate::test_utils::spawn_mock_server;
    use knot_proto::api::v1::daemon_service_client::DaemonServiceClient;
    use knot_proto::commands;
    use knot_proto::commands::v1::{SyncResponse, SyncResult};
    use knot_proto::data::v1::{WorkspaceManifest, WorkspaceMetadata};
    use std::sync::Arc;
    use tokio_stream::{StreamExt, wrappers::ReceiverStream};
    use tonic::transport::Channel;
    use tonic::{Code, Response};

    const WORKSPACE_ID: &str = "test_id";
    const WORKSPACE_ROOT: &str = "/test/path";
    const COMMAND_ID: &str = "cmd-sync-abc123";

    fn controller(client: DaemonServiceClient<Channel>) -> ControlHandle {
        ControlHandle {
            workspace_meta: WorkspaceMetadata {
                workspace_id: WORKSPACE_ID.to_string(),
                root_path: WORKSPACE_ROOT.to_string(),
            },
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    fn manifest() -> WorkspaceManifest {
        WorkspaceManifest::default()
    }

    fn sync_response(
        command_id: Option<&str>,
        events: Vec<SyncResponse>,
    ) -> Response<ReceiverStream<Result<SyncResponse, tonic::Status>>> {
        let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));

        for event in events {
            tx.try_send(Ok(event))
                .expect("mock sync stream should accept event");
        }

        let mut response = Response::new(ReceiverStream::new(rx));

        if let Some(command_id) = command_id {
            response
                .metadata_mut()
                .insert("x-command-id", command_id.parse().unwrap());
        }

        response
    }

    fn sync_result(service: &str) -> SyncResponse {
        SyncResponse {
            event: Some(commands::v1::sync_response::Event::Result(SyncResult {
                services_added: vec![service.to_string()],
                services_changed: vec![],
                services_removed: vec![],
            })),
        }
    }

    #[tokio::test]
    async fn sync_sends_workspace_manifest_and_metadata() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|request| {
                let request = request.into_inner();

                let metadata = request
                    .metadata
                    .expect("sync request must contain workspace metadata");

                assert_eq!(metadata.workspace_id, WORKSPACE_ID);
                assert_eq!(metadata.root_path, WORKSPACE_ROOT);

                assert!(
                    request.manifest.is_some(),
                    "sync request must contain workspace manifest"
                );

                Ok(sync_response(
                    Some(COMMAND_ID),
                    vec![sync_result("service-a")],
                ))
            }));
        }

        let (_controller, mut command) =
            handle.sync(manifest()).await.expect("sync should succeed");

        assert_eq!(command.command_id, COMMAND_ID);

        let event = command
            .next()
            .await
            .expect("expected sync event")
            .expect("sync stream returned an error");

        assert!(matches!(
            event.event,
            Some(commands::v1::sync_response::Event::Result(_))
        ));
    }

    #[tokio::test]
    async fn sync_returns_command_handle_with_command_id() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Ok(sync_response(Some("sync-command-42"), vec![]))
            }));
        }

        let (_controller, command) = handle.sync(manifest()).await.expect("sync should succeed");

        assert_eq!(command.command_id, "sync-command-42");
    }

    #[tokio::test]
    async fn sync_preserves_all_stream_events() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Ok(sync_response(
                    Some(COMMAND_ID),
                    vec![
                        sync_result("service-a"),
                        sync_result("service-b"),
                        sync_result("service-c"),
                    ],
                ))
            }));
        }

        let (_controller, mut command) =
            handle.sync(manifest()).await.expect("sync should succeed");

        let mut events = Vec::new();

        while let Some(event) = command.next().await {
            events.push(event.expect("sync stream returned an error"));
        }

        assert_eq!(events.len(), 3);

        for (event, expected_service) in events.iter().zip(["service-a", "service-b", "service-c"])
        {
            match &event.event {
                Some(commands::v1::sync_response::Event::Result(result)) => {
                    assert_eq!(result.services_added, vec![expected_service]);
                }

                other => {
                    panic!("expected SyncResult event, got {other:?}");
                }
            }
        }
    }

    #[tokio::test]
    async fn sync_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Err(tonic::Status::failed_precondition(
                    "workspace cannot be synchronized",
                ))
            }));
        }

        let result = handle.sync(manifest()).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::FailedPrecondition
        ),);
    }

    #[tokio::test]
    async fn sync_returns_contract_error_when_command_id_is_missing() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                // Deliberately omit x-command-id.
                Ok(sync_response(None, vec![]))
            }));
        }

        let result = handle.sync(manifest()).await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message.contains("x-command-id")
        ),);
    }

    #[tokio::test]
    async fn sync_returns_original_controller() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UnsyncedHandle { controller };

        {
            let mut handler = mock.sync_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Ok(sync_response(Some(COMMAND_ID), vec![]))
            }));
        }

        let (returned_controller, command) =
            handle.sync(manifest()).await.expect("sync should succeed");

        assert_eq!(
            returned_controller.workspace_meta.workspace_id,
            WORKSPACE_ID
        );

        assert_eq!(returned_controller.workspace_meta.root_path, WORKSPACE_ROOT);

        assert_eq!(command.command_id, COMMAND_ID);
    }
}
