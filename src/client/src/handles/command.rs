use knot_proto::{
    api::v1::daemon_service_client::DaemonServiceClient
    ,
    command::v1::CancelCommandRequest};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_stream::Stream;
use tonic::{Streaming, transport::Channel};
use tracing::{debug, error, info, instrument};

/// An asynchronous stream wrapper representing a long-running daemon command.
///
/// `CommandHandle` implements the `Stream` trait, allowing you to seamlessly iterate
/// over real-time execution events (such as task progress or completion statuses).
/// It also retains a clone of the underlying gRPC client, empowering the caller to
/// programmatically abort the command mid-execution via the [`Self::cancel`] method.
#[derive(Debug)]
pub struct CommandHandle<E> {
    /// The unique identifier assigned by the daemon for this specific command execution.
    pub command_id: String,
    pub(crate) events: Streaming<E>,
    client: DaemonServiceClient<Channel>,
}

impl<E> CommandHandle<E> {
    /// Constructs a new `CommandHandle`.
    ///
    /// This is typically called internally by the orchestrator after initiating a
    /// command like `up` or `down`.
    pub fn new(
        command_id: String,
        events: Streaming<E>,
        client: DaemonServiceClient<Channel>,
    ) -> Self {
        Self {
            command_id,
            events,
            client,
        }
    }

    /// Attempts to gracefully abort the ongoing command execution on the daemon side.
    ///
    /// This method sends a cancellation signal to the daemon. If the command is still running,
    /// the daemon will attempt to halt further task execution and initiate rollback or shutdown
    /// procedures for tasks spawned by this specific command ID.
    ///
    /// # Arguments
    ///
    /// * `reason` - A descriptive reason for the cancellation (e.g., "user pressed Ctrl+C").
    ///
    /// # Returns
    ///
    /// Returns `true` if the daemon successfully caught and cancelled the command, or `false`
    /// if the command had already finished or could not be cancelled.
    ///
    /// # Errors
    ///
    /// Returns a `tonic::Status` if the gRPC network request fails or if the daemon is unreachable.
    #[instrument(skip(self, reason), fields(command_id = %self.command_id))]
    pub async fn cancel(&self, reason: impl Into<String>) -> Result<bool, tonic::Status> {
        let reason_str = reason.into();
        debug!(reason = %reason_str, "sending cancellation request for active command");

        let mut client = self.client.clone();
        let resp = client
            .cancel_command(CancelCommandRequest {
                command_id: self.command_id.clone(),
                reason: Some(reason_str),
            })
            .await
            .map_err(|e| {
                error!(error = %e, "failed to send cancellation request to daemon");
                e
            })?;

        let is_cancelled = resp.into_inner().cancelled;
        if is_cancelled {
            info!("command successfully cancelled by the daemon");
        } else {
            debug!("daemon rejected cancellation (command may have already completed)");
        }

        Ok(is_cancelled)
    }
}

impl<E> Stream for CommandHandle<E> {
    type Item = Result<E, tonic::Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Delegate the polling directly to the underlying tonic::Streaming wrapper.
        Pin::new(&mut self.events).poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::spawn_mock_server;
    use knot_proto::{
        api::v1::daemon_service_client::DaemonServiceClient,
        command::v1::CancelCommandResponse,
        commands::v1::{
            UpRequest,
            UpResponse,
        },
    };
    use tonic::{Code, Response};

    const WORKSPACE_ID: &str = "test-workspace";

    async fn create_up_handle(
        mock: &crate::test_utils::MockKnotDaemon,
        client: DaemonServiceClient<Channel>,
        command_id: &str,
    ) -> CommandHandle<UpResponse> {
        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(
                    request.workspace_id,
                    WORKSPACE_ID,
                    "up command must contain workspace_id"
                );

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                Ok(Response::new(
                    tokio_stream::wrappers::ReceiverStream::new(rx),
                ))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .unwrap();

        CommandHandle::new(
            command_id.to_string(),
            response.into_inner(),
            client,
        )
    }

    #[tokio::test]
    async fn test_cancel_command_success() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-1");
                assert_eq!(
                    request.reason.as_deref(),
                    Some("user abort")
                );

                Ok(Response::new(CancelCommandResponse {
                    cancelled: true,
                }))
            }));
        }

        let handle = create_up_handle(
            &mock,
            client,
            "test-cmd-id-1",
        )
            .await;

        let cancelled = handle
            .cancel("user abort")
            .await
            .expect("cancel should succeed");

        assert!(cancelled);
    }

    #[tokio::test]
    async fn test_cancel_command_already_completed() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-2");
                assert_eq!(
                    request.reason.as_deref(),
                    Some("already done")
                );

                Ok(Response::new(CancelCommandResponse {
                    cancelled: false,
                }))
            }));
        }

        let handle = create_up_handle(
            &mock,
            client,
            "test-cmd-id-2",
        )
            .await;

        let cancelled = handle
            .cancel("already done")
            .await
            .expect("cancel should succeed");

        assert!(!cancelled);
    }

    #[tokio::test]
    async fn test_cancel_command_grpc_error_is_preserved() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-3");
                assert_eq!(
                    request.reason.as_deref(),
                    Some("daemon busy")
                );

                Err(tonic::Status::unavailable(
                    "daemon unavailable",
                ))
            }));
        }

        let handle = create_up_handle(
            &mock,
            client,
            "test-cmd-id-3",
        )
            .await;

        let result = handle.cancel("daemon busy").await;

        assert!(
            matches!(
                result.clone(),
                Err(status) if status.code() == Code::Unavailable
            ),
            "unexpected result: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_command_handle_stream_yields_responses_in_order() {
        use tokio_stream::StreamExt;

        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, WORKSPACE_ID);

                let (tx, rx) = tokio::sync::mpsc::channel(2);

                tx.try_send(Ok(UpResponse::default()))
                    .unwrap();

                tx.try_send(Ok(UpResponse::default()))
                    .unwrap();

                Ok(Response::new(
                    tokio_stream::wrappers::ReceiverStream::new(rx),
                ))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .unwrap();

        let mut handle: CommandHandle<UpResponse> =
            CommandHandle::new(
                "stream-cmd".to_string(),
                response.into_inner(),
                client,
            );

        assert!(handle.next().await.unwrap().is_ok());
        assert!(handle.next().await.unwrap().is_ok());
        assert!(handle.next().await.is_none());
    }

    #[tokio::test]
    async fn test_command_handle_preserves_command_id() {
        let (mock, client) = spawn_mock_server().await;

        let handle = create_up_handle(
            &mock,
            client,
            "preserved-command-id",
        )
            .await;

        assert_eq!(
            handle.command_id,
            "preserved-command-id"
        );
    }
}
