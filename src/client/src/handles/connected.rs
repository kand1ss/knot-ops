use crate::{
    errors::ClientError,
    handles::{ControlHandle, UnsyncedHandle},
    policies::PolicyConfig,
    states::DaemonSession,
    utils::request
};
use knot_proto::{
    api::v1::daemon_service_client::DaemonServiceClient,
    commands::v1::HandshakeRequest,
    data::v1::{WorkspaceManifest, WorkspaceMetadata},
};
use std::{path::Path, sync::Arc};
use tokio::time::Duration;
use tonic::{
    Request,
    transport::{Channel, Endpoint},
};
use tracing::{debug, error, instrument};

use knot_grpc::IpcConnector;
use knot_proto::data::v1::WorkspaceState;

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
        workspace_meta: WorkspaceMetadata,
        workspace_manifest: WorkspaceManifest,
    ) -> Result<DaemonSession, ClientError> {
        let mut client = self.client.clone();
        let response = client
            .handshake(request(
                HandshakeRequest {
                    metadata: Some(workspace_meta.clone()),
                    manifest: Some(workspace_manifest),
                },
                Some(self.policy.timeout.fast_commands),
            ))
            .await?;

        let controller = ControlHandle {
            client,
            workspace_meta,
            policy: Arc::clone(&self.policy),
        };
        let res = response.into_inner();

        match WorkspaceState::try_from(res.state) {
            Ok(WorkspaceState::OutOfSync) => {
                let handle = UnsyncedHandle { controller };
                Ok(DaemonSession::Unsynced(handle))
            }
            Ok(WorkspaceState::InSync) => Ok(DaemonSession::Ready(controller)),
            Ok(WorkspaceState::Unregistered) => Err(ClientError::Contract(
                "workspace registration error".to_string(),
            )),
            Ok(WorkspaceState::Unspecified) | Err(_) => {
                Err(ClientError::Contract("unknown workspace state".to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::spawn_mock_server;
    use knot_proto::{
        api::v1::daemon_service_client::DaemonServiceClient, commands::v1::HandshakeResponse,
        data::v1::WorkspaceState,
    };
    use tonic::{Code, Response};

    fn handle(client: DaemonServiceClient<Channel>) -> ConnectedHandle {
        ConnectedHandle {
            client,
            policy: Arc::new(PolicyConfig::default()),
        }
    }

    fn workspace_metadata() -> WorkspaceMetadata {
        WorkspaceMetadata {
            workspace_id: "workspace-test".to_string(),
            root_path: "/tmp/knot-test".to_string(),
        }
    }

    fn workspace_manifest() -> WorkspaceManifest {
        WorkspaceManifest::default()
    }

    #[tokio::test]
    async fn handshake_returns_unsynced_for_out_of_sync_workspace() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: WorkspaceState::OutOfSync as i32,
                    ..Default::default()
                }))
            }));
        }

        let session = handle
            .handshake(workspace_metadata(), workspace_manifest())
            .await
            .unwrap();

        assert!(matches!(session, DaemonSession::Unsynced(_)));
    }

    #[tokio::test]
    async fn handshake_returns_ready_for_in_sync_workspace() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Ok(Response::new(HandshakeResponse {
                    state: WorkspaceState::InSync as i32,
                    ..Default::default()
                }))
            }));
        }

        let session = handle
            .handshake(workspace_metadata(), workspace_manifest())
            .await
            .unwrap();

        assert!(matches!(session, DaemonSession::Ready(_)));
    }

    #[tokio::test]
    async fn handshake_sends_workspace_metadata_and_manifest() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                let metadata = request
                    .metadata
                    .expect("handshake must contain workspace metadata");

                assert_eq!(metadata.workspace_id, "workspace-test");
                assert_eq!(metadata.root_path, "/tmp/knot-test");

                assert!(
                    request.manifest.is_some(),
                    "handshake must contain workspace manifest"
                );

                Ok(Response::new(HandshakeResponse {
                    state: WorkspaceState::InSync as i32,
                    ..Default::default()
                }))
            }));
        }

        let session = handle
            .handshake(workspace_metadata(), workspace_manifest())
            .await
            .unwrap();

        assert!(matches!(session, DaemonSession::Ready(_)));
    }

    #[tokio::test]
    async fn handshake_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;
        let handle = handle(client);

        {
            let mut handler = mock.handshake_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                Err(tonic::Status::unavailable("daemon unavailable"))
            }));
        }

        let result = handle
            .handshake(workspace_metadata(), workspace_manifest())
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
                    state: WorkspaceState::Unregistered as i32,
                    ..Default::default()
                }))
            }));
        }

        let result = handle
            .handshake(workspace_metadata(), workspace_manifest())
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
                    state: WorkspaceState::Unspecified as i32,
                    ..Default::default()
                }))
            }));
        }

        let result = handle
            .handshake(workspace_metadata(), workspace_manifest())
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
                    ..Default::default()
                }))
            }));
        }

        let result = handle
            .handshake(workspace_metadata(), workspace_manifest())
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
                    state: WorkspaceState::InSync as i32,
                    ..Default::default()
                }))
            }));
        }

        let session = handle
            .handshake(
                WorkspaceMetadata {
                    workspace_id: "my-workspace".to_string(),
                    root_path: "/home/test/project".to_string(),
                },
                workspace_manifest(),
            )
            .await
            .unwrap();

        match session {
            DaemonSession::Ready(controller) => {
                assert_eq!(controller.workspace_meta.workspace_id, "my-workspace");
                assert_eq!(controller.workspace_meta.root_path, "/home/test/project");
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
                    state: WorkspaceState::OutOfSync as i32,
                    ..Default::default()
                }))
            }));
        }

        let session = handle
            .handshake(
                WorkspaceMetadata {
                    workspace_id: "out-of-sync".to_string(),
                    root_path: "/workspace/project".to_string(),
                },
                workspace_manifest(),
            )
            .await
            .unwrap();

        match session {
            DaemonSession::Unsynced(unsynced) => {
                assert_eq!(
                    unsynced.controller.workspace_meta.workspace_id,
                    "out-of-sync"
                );

                assert_eq!(
                    unsynced.controller.workspace_meta.root_path,
                    "/workspace/project"
                );
            }

            _ => panic!("expected Unsynced session"),
        }
    }
}
