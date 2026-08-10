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

        let response = self.controller.sync(workspace_manifest).await.map_err(|e| {
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
    use knot_proto::data::v1::WorkspaceMetadata;
    use std::sync::Arc;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::Response;
    use tonic::transport::Channel;
    use knot_proto::commands::v1::SyncResult;

    fn handle(client: DaemonServiceClient<Channel>) -> ControlHandle {
        ControlHandle {
            workspace_meta: WorkspaceMetadata {
                workspace_id: "test_id".to_string(),
                root_path: "/test/path".to_string(),
            },
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    #[tokio::test]
    async fn test_uninitialized_sync() {
        let (mock, client) = spawn_mock_server().await;
        let controller = handle(client);
        let handle = UnsyncedHandle { controller };
        {
            let mut handler = mock.sync_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                let mut response = Response::new(ReceiverStream::new(rx));
                response.metadata_mut().insert("x-command-id", "cmd-sync-abc123".parse().unwrap());
                tx.try_send(Ok(SyncResponse {
                    event: Some(commands::v1::sync_response::Event::Result(SyncResult {
                        services_added: vec!["test_service".to_string()],
                        services_changed: vec![],
                        services_removed: vec![],
                    }))
                })).unwrap();
                Ok(response)
            }));
        }

        let (returned_controller, mut sync_resp) = handle.sync(WorkspaceManifest::default()).await.unwrap();

        let mut found = false;

        while let Some(event) = sync_resp.events.message().await.unwrap() {
            match event.event {
                Some(commands::v1::sync_response::Event::Result(sync_result)) => {
                    assert_eq!(sync_result.services_added.len(), 1);
                    found = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(
            found,
            "expected SyncResult event in sync event stream"
        );

        let _ = returned_controller;
    }

    #[tokio::test]
    async fn test_uninitialized_sync_error() {
        let (mock, client) = spawn_mock_server().await;
        let controller = handle(client);
        let handle = UnsyncedHandle { controller };
        {
            let mut handler = mock.sync_handler.lock().await;
            *handler = Some(Box::new(|_req| {
                Err(tonic::Status::internal("Daemon failed"))
            }));
        }

        let result = handle.sync(WorkspaceManifest::default()).await;

        assert!(result.is_err());
    }
}
