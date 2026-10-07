use crate::errors::ClientError;
use crate::handles::ControlHandle;
use crate::utils::request;
use knot_proto::v1::{
    commands::{CommitRequest, CommitResponse},
    config::WorkspaceManifest,
};
use tracing::{debug, error, instrument};

/// Outcome of a successful commit.
#[derive(Debug)]
#[non_exhaustive] // allows adding fields without a breaking change
pub struct Committed {
    /// Session in the in-sync state; ready for `up` / `down` / `status`.
    pub control: ControlHandle,
    /// Service-level changes the daemon applied.
    pub summary: CommitResponse,
}

/// A failed commit. Carries the handle back so the caller can retry or `skip()`.
#[derive(Debug, thiserror::Error)]
#[error("failed to commit workspace changes")]
pub struct CommitError {
    pub handle: Box<UncommittedHandle>,
    #[source]
    pub source: ClientError,
}

// Keeps `?` ergonomic for callers that do not care about recovery.
impl From<CommitError> for ClientError {
    fn from(e: CommitError) -> Self {
        e.source
    }
}

/// A stateful handle representing a connected but unconfigured daemon.
///
/// This handle is returned during the handshake phase if the daemon is currently running
/// but has no active workspace configuration loaded in memory. In this state, lifecycle
/// commands (such as `up` or `down`) are invalid. The only permitted operational action is
/// to provide the initial configuration via the [`Self::sync`] method.
#[derive(Debug)]
pub struct UncommittedHandle {
    pub(crate) controller: ControlHandle,
    pub(crate) to_commit: WorkspaceManifest,
}

impl UncommittedHandle {
    /// Sends the pending workspace changes to the daemon.
    ///
    /// Consumes the handle. On success returns the in-sync [`ControlHandle`]
    /// together with the daemon's summary of applied changes.
    ///
    /// # Errors
    ///
    /// On failure the handle is returned inside [`CommitError`]. Retrying is
    /// safe only because the daemon treats a commit of an already-applied
    /// manifest as a no-op.
    #[instrument(skip_all, name = "commit")]
    pub async fn commit(self) -> Result<Committed, CommitError> {
        match self.inner_commit().await {
            Ok(summary) => Ok(Committed {
                control: self.controller,
                summary,
            }),
            Err(source) => Err(CommitError {
                handle: Box::new(self),
                source,
            }),
        }
    }

    async fn inner_commit(&self) -> Result<CommitResponse, ClientError> {
        debug!("sending 'commit' request to daemon");

        let mut client = self.controller.client.clone();
        let response = client
            .commit(request(
                CommitRequest {
                    workspace_id: self.controller.workspace_id.clone(),
                    expected_revision: self.controller.expected_revision.clone(),
                    manifest: Some(self.to_commit.clone()),
                },
                Some(self.controller.policy.timeout.fast_commands),
            ))
            .await
            .map_err(|e| {
                error!(error = %e, "failed to commit workspace configuration");
                e
            })?;
        Ok(response.into_inner())
    }

    /// Abandons the pending commit and continues with the daemon's current state.
    ///
    /// Consumes the handle and returns the underlying [`ControlHandle`] **without**
    /// contacting the daemon. The changes held by this handle are discarded and are
    /// not sent anywhere.
    ///
    /// # Caveats
    ///
    /// The returned [`ControlHandle`] operates on the configuration the daemon
    /// already has, which is out of sync with the local workspace. Commands such as
    /// `up` will act on the daemon's last committed state, not on local changes.
    /// Use this only when running against the stale configuration is intended
    /// (e.g. the user explicitly opted out of synchronization).
    ///
    /// This method performs no I/O and cannot fail.
    pub fn discard(self) -> ControlHandle {
        self.controller
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
    const REVISION: &str = "test";

    fn controller(client: DaemonServiceClient<Channel>) -> ControlHandle {
        ControlHandle {
            expected_revision: REVISION.to_string(),
            workspace_id: WORKSPACE_ID.to_string(),
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    fn handle(client: DaemonServiceClient<Channel>) -> UncommittedHandle {
        UncommittedHandle {
            controller: controller(client),
            to_commit: manifest(),
        }
    }

    fn manifest() -> WorkspaceManifest {
        WorkspaceManifest::default()
    }

    fn commit_response(service: &str) -> CommitResponse {
        CommitResponse {
            revision: REVISION.to_string(),
            services_added: vec![service.to_string()],
            services_changed: vec![],
            services_removed: vec![],
        }
    }

    #[tokio::test]
    async fn commit_sends_workspace_manifest_and_id() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

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

        let response = handle.commit().await.expect("sync should succeed");

        assert_eq!(
            response.summary.services_added,
            vec!["service-a".to_string()]
        );
    }

    #[tokio::test]
    async fn commit_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|_request| {
                Err(tonic::Status::failed_precondition(
                    "workspace cannot be synchronized",
                ))
            }));
        }

        let result = handle.commit().await;
        let error = result.unwrap_err();

        assert!(matches!(
            error.source,
            ClientError::Protocol(status)
                if status.code() == Code::FailedPrecondition
        ),);
    }

    #[tokio::test]
    async fn commit_returns_original_controller() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.commit_handler.lock().await;

            *handler = Some(Box::new(|_request| Ok(Response::new(commit_response("")))));
        }

        let response = handle.commit().await.expect("sync should succeed");

        assert_eq!(response.control.workspace_id, WORKSPACE_ID);
    }
}
