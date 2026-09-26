use crate::errors::ClientError;
use crate::handles::ControlHandle;
use knot_proto::v1::{commands::CommitResponse, config::WorkspaceManifest};
use tracing::{debug, error, info, instrument};

/// A stateful handle representing a connected but unconfigured daemon.
///
/// This handle is returned during the handshake phase if the daemon is currently running
/// but has no active workspace configuration loaded in memory. In this state, lifecycle
/// commands (such as `up` or `down`) are invalid. The only permitted operational action is
/// to provide the initial configuration via the [`Self::sync`] method.
#[derive(Debug)]
pub struct UncommitedHandle {
    pub(crate) controller: ControlHandle,
}

impl UncommitedHandle {
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
    pub async fn commit(
        self,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<(ControlHandle, CommitResponse), ClientError> {
        debug!("pushing initial workspace configuration to uninitialized daemon");

        let response = self
            .controller
            .commit(workspace_manifest)
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
    use knot_proto::v1::{
        commands::CommitResponse, config::WorkspaceManifest,
        daemon_service_client::DaemonServiceClient,
    };
    use std::sync::Arc;

    use tonic::transport::Channel;
    use tonic::{Code, Response};

    const WORKSPACE_ID: &str = "test_id";

    fn controller(client: DaemonServiceClient<Channel>) -> ControlHandle {
        ControlHandle {
            workspace_id: WORKSPACE_ID.to_string(),
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    fn manifest() -> WorkspaceManifest {
        WorkspaceManifest::default()
    }

    fn commit_response(service: &str) -> CommitResponse {
        CommitResponse {
            services_added: vec![service.to_string()],
            services_changed: vec![],
            services_removed: vec![],
        }
    }

    #[tokio::test]
    async fn commit_sends_workspace_manifest_and_id() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UncommitedHandle { controller };

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|request| {
                let request = request.into_inner();

                let metadata = request.workspace_id;
                assert_eq!(metadata, WORKSPACE_ID);
                assert!(
                    request.manifest.is_some(),
                    "sync request must contain workspace manifest"
                );

                Ok(Response::new(commit_response("service-a")))
            }));
        }

        let (_controller, response) = handle
            .commit(manifest())
            .await
            .expect("sync should succeed");

        assert_eq!(response.services_added, vec!["service-a".to_string()]);
    }

    #[tokio::test]
    async fn commit_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UncommitedHandle { controller };

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Err(tonic::Status::failed_precondition(
                    "workspace cannot be synchronized",
                ))
            }));
        }

        let result = handle.commit(manifest()).await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::FailedPrecondition
        ),);
    }

    #[tokio::test]
    async fn commit_returns_original_controller() {
        let (mock, client) = spawn_mock_server().await;

        let controller = controller(client);
        let handle = UncommitedHandle { controller };

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|_request| Ok(Response::new(commit_response("")))));
        }

        let (returned_controller, _command) = handle
            .commit(manifest())
            .await
            .expect("sync should succeed");

        assert_eq!(returned_controller.workspace_id, WORKSPACE_ID);
    }
}
