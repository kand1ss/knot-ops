use crate::{
    errors::ClientError,
    handles::{ControlHandle, UncommitedHandle},
    policies::PolicyConfig,
    states::DaemonSession,
    utils::request,
};
use knot_proto::v1::{
    commands::HandshakeRequest, config::WorkspaceManifest,
    daemon_service_client::DaemonServiceClient,
};
use std::{path::Path, sync::Arc};
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, error, instrument};

use knot_grpc::IpcConnector;
use knot_proto::v1::commands::handshake_response::ManifestSyncState;

#[derive(Debug)]
pub struct ConnectedHandle {
    pub(crate) client: DaemonServiceClient<Channel>,
    pub(crate) policy: Arc<PolicyConfig>,
}

impl ConnectedHandle {
    /// Establishes a new IPC connection to the Knot daemon.
    ///
    /// # Arguments
    ///
    /// * `socket_path` - The file system path to the UNIX domain socket (or named pipe).
    ///
    /// # Errors
    ///
    /// Returns a `ClientError` if the channel cannot be established.
    #[instrument(skip(socket_path), fields(socket = %socket_path.display()))]
    pub(crate) async fn new(
        socket_path: &Path,
        policy: Arc<PolicyConfig>,
    ) -> Result<Self, ClientError> {
        debug!("attempting to connect to the knot daemon");

        let connector = IpcConnector::new(socket_path);
        let channel = Endpoint::try_from("http://[::]")?
            .connect_with_connector(connector)
            .await
            .map_err(|e| {
                error!(error = %e, "failed to connect to IPC socket");
                e
            })?;

        debug!("successfully established gRPC channel over IPC");
        let client = DaemonServiceClient::new(channel);
        Ok(Self { client, policy })
    }

    pub async fn handshake(
        self,
        workspace_id: String,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<DaemonSession, ClientError> {
        let mut client = self.client.clone();
        let response = client
            .handshake(request(
                HandshakeRequest {
                    workspace_id: workspace_id.clone(),
                    manifest: Some(workspace_manifest),
                },
                Some(self.policy.timeout.fast_commands),
            ))
            .await?;

        let controller = ControlHandle {
            client,
            workspace_id,
            policy: Arc::clone(&self.policy),
        };
        let res = response.into_inner();

        match ManifestSyncState::try_from(res.state) {
            Ok(ManifestSyncState::OutOfSync) => {
                let handle = UncommitedHandle { controller };
                Ok(DaemonSession::Unsynced(handle))
            }
            Ok(ManifestSyncState::InSync) => Ok(DaemonSession::Ready(controller)),
            Ok(ManifestSyncState::Unregistered) => Err(ClientError::Contract(
                "workspace registration error".to_string(),
            )),
            Ok(ManifestSyncState::Unspecified) | Err(_) => {
                Err(ClientError::Contract("unknown workspace state".to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::spawn_mock_server;

    use knot_proto::v1::{
        commands::{HandshakeRequest, HandshakeResponse},
        config::WorkspaceManifest,
        daemon_service_client::DaemonServiceClient,
    };

    use std::sync::Arc;
    use tonic::{Code, Request, Response, Status};

    fn handle(client: DaemonServiceClient<Channel>) -> ConnectedHandle {
        ConnectedHandle {
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    fn workspace_manifest() -> WorkspaceManifest {
        WorkspaceManifest::default()
    }

    fn custom_workspace_manifest() -> WorkspaceManifest {
        WorkspaceManifest {
            // Keep this function populated with the fields that exist
            // in the current protobuf definition.
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn handshake_returns_unsynced_for_out_of_sync_workspace() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::OutOfSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        assert!(
            matches!(session, DaemonSession::Unsynced(_)),
            "expected Unsynced session"
        );
    }

    #[tokio::test]
    async fn handshake_returns_ready_for_in_sync_workspace() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::InSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        assert!(
            matches!(session, DaemonSession::Ready(_)),
            "expected Ready session"
        );
    }

    #[tokio::test]
    async fn handshake_sends_complete_workspace_metadata() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|req: Request<HandshakeRequest>| {
                let request = req.into_inner();

                let workspace_id = request.workspace_id;
                assert_eq!(workspace_id, "workspace-test");

                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::InSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        assert!(matches!(session, DaemonSession::Ready(_)));
    }

    #[tokio::test]
    async fn handshake_sends_workspace_manifest() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        let expected_manifest = custom_workspace_manifest();

        {
            let expected_manifest = expected_manifest.clone();
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(move |req| {
                let request = req.into_inner();

                let actual_manifest = request
                    .manifest
                    .expect("handshake must contain workspace manifest");

                assert_eq!(actual_manifest, expected_manifest);

                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::InSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), expected_manifest)
            .await
            .expect("handshake should succeed");

        assert!(matches!(session, DaemonSession::Ready(_)));
    }

    #[tokio::test]
    async fn handshake_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        assert!(matches!(
            result,
            Err(ClientError::Protocol(status))
                if status.code() == Code::Unavailable
        ),);
    }

    #[tokio::test]
    async fn handshake_rejects_unregistered_workspace() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::Unregistered as i32,
                }))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message == "workspace registration error"
        ),);
    }

    #[tokio::test]
    async fn handshake_rejects_unspecified_workspace_state() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::Unspecified as i32,
                }))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message == "unknown workspace state"
        ),);
    }

    #[tokio::test]
    async fn handshake_rejects_unknown_workspace_state() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    // Deliberately invalid protobuf enum value.
                    state: 999,
                }))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message == "unknown workspace state"
        ),);
    }

    #[tokio::test]
    async fn handshake_preserves_workspace_metadata_in_ready_controller() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::InSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("my-workspace".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        match session {
            DaemonSession::Ready(controller) => {
                assert_eq!(controller.workspace_id, "my-workspace");
            }

            _ => panic!("expected Ready session"),
        }
    }

    #[tokio::test]
    async fn handshake_preserves_workspace_metadata_in_unsynced_controller() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::OutOfSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("out-of-sync".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        match session {
            DaemonSession::Unsynced(unsynced) => {
                assert_eq!(unsynced.controller.workspace_id, "out-of-sync");
            }

            _ => panic!("expected Unsynced session"),
        }
    }

    #[tokio::test]
    async fn handshake_preserves_policy_in_ready_controller() {
        let (mock, client) = spawn_mock_server().await;

        let policy = Arc::new(PolicyConfig::default());

        let handle = ConnectedHandle {
            client,
            policy: Arc::clone(&policy),
        };

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::InSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        match session {
            DaemonSession::Ready(controller) => {
                assert!(
                    Arc::ptr_eq(&controller.policy, &policy),
                    "policy Arc must be preserved"
                );
            }

            _ => panic!("expected Ready session"),
        }
    }

    #[tokio::test]
    async fn handshake_preserves_policy_in_unsynced_controller() {
        let (mock, client) = spawn_mock_server().await;

        let policy = Arc::new(PolicyConfig::default());

        let handle = ConnectedHandle {
            client,
            policy: Arc::clone(&policy),
        };

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: ManifestSyncState::OutOfSync as i32,
                }))
            }));
        }

        let session = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await
            .expect("handshake should succeed");

        match session {
            DaemonSession::Unsynced(unsynced) => {
                assert!(
                    Arc::ptr_eq(&unsynced.controller.policy, &policy),
                    "policy Arc must be preserved"
                );
            }

            _ => panic!("expected Unsynced session"),
        }
    }

    #[tokio::test]
    async fn handshake_propagates_internal_server_error() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(Status::internal("internal daemon failure"))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        match result {
            Err(ClientError::Protocol(status)) => {
                assert_eq!(status.code(), Code::Internal);

                assert_eq!(status.message(), "internal daemon failure");
            }

            _ => {
                panic!("expected ClientError::Protocol");
            }
        }
    }

    #[tokio::test]
    async fn handshake_rejects_missing_response_state() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse::default()))
            }));
        }

        let result = handle
            .handshake("workspace-test".to_string(), workspace_manifest())
            .await;

        assert!(matches!(
            result,
            Err(ClientError::Contract(message))
                if message == "unknown workspace state"
        ),);
    }
}
