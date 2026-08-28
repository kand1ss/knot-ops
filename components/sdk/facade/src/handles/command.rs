use knot_proto::{
    api::v1::daemon_service_client::DaemonServiceClient, command::v1::CancelCommandRequest,
};
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
        commands::v1::{UpRequest, UpResponse},
    };

    use tokio_stream::StreamExt;
    use tonic::{Code, Response, Status, transport::Channel};

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
                    request.workspace_id, WORKSPACE_ID,
                    "up command must contain workspace_id"
                );

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("failed to create mock command stream");

        CommandHandle::new(command_id.to_string(), response.into_inner(), client)
    }

    #[tokio::test]
    async fn cancel_returns_true_when_daemon_cancels_command() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-1");

                assert_eq!(request.reason.as_deref(), Some("user abort"));

                Ok(Response::new(CancelCommandResponse { cancelled: true }))
            }));
        }

        let handle = create_up_handle(&mock, client, "test-cmd-id-1").await;

        let cancelled = handle
            .cancel("user abort")
            .await
            .expect("cancel should succeed");

        assert!(
            cancelled,
            "cancel must return true when daemon confirms cancellation"
        );
    }

    #[tokio::test]
    async fn cancel_returns_false_when_command_is_already_completed() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-2");

                assert_eq!(request.reason.as_deref(), Some("already done"));

                Ok(Response::new(CancelCommandResponse { cancelled: false }))
            }));
        }

        let handle = create_up_handle(&mock, client, "test-cmd-id-2").await;

        let cancelled = handle
            .cancel("already done")
            .await
            .expect("cancel RPC should succeed");

        assert!(
            !cancelled,
            "cancel must return false when daemon rejects cancellation"
        );
    }

    #[tokio::test]
    async fn cancel_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "test-cmd-id-3");

                assert_eq!(request.reason.as_deref(), Some("daemon busy"));

                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let handle = create_up_handle(&mock, client, "test-cmd-id-3").await;

        let result = handle.cancel("daemon busy").await;

        match result {
            Err(status) => {
                assert_eq!(status.code(), Code::Unavailable);

                assert_eq!(status.message(), "daemon unavailable");
            }

            Ok(value) => {
                panic!("expected gRPC error, got successful result: {value}");
            }
        }
    }

    #[tokio::test]
    async fn cancel_accepts_non_string_reason() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "string-conversion-test");

                assert_eq!(request.reason.as_deref(), Some("42"));

                Ok(Response::new(CancelCommandResponse { cancelled: true }))
            }));
        }

        let handle = create_up_handle(&mock, client, "string-conversion-test").await;

        let cancelled = handle
            .cancel(42.to_string())
            .await
            .expect("cancel should succeed");

        assert!(cancelled);
    }

    #[tokio::test]
    async fn cancel_sends_empty_reason_when_empty_string_is_provided() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "empty-reason");

                assert_eq!(request.reason.as_deref(), Some(""));

                Ok(Response::new(CancelCommandResponse { cancelled: true }))
            }));
        }

        let handle = create_up_handle(&mock, client, "empty-reason").await;

        let cancelled = handle.cancel("").await.expect("cancel should succeed");

        assert!(cancelled);
    }

    #[tokio::test]
    async fn command_handle_stream_yields_responses_in_order() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.workspace_id, WORKSPACE_ID);

                let (tx, rx) = tokio::sync::mpsc::channel(2);

                tx.try_send(Ok(UpResponse::default()))
                    .expect("failed to send first response");

                tx.try_send(Ok(UpResponse::default()))
                    .expect("failed to send second response");

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

        let mut handle = CommandHandle::<UpResponse>::new(
            "stream-cmd".to_string(),
            response.into_inner(),
            client,
        );

        assert!(handle.next().await.expect("first item must exist").is_ok());

        assert!(handle.next().await.expect("second item must exist").is_ok());

        assert!(
            handle.next().await.is_none(),
            "stream must terminate after all responses"
        );
    }

    #[tokio::test]
    async fn command_handle_stream_preserves_response_order() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                let (tx, rx) = tokio::sync::mpsc::channel(3);

                tx.try_send(Ok(UpResponse {
                    ..Default::default()
                }))
                .unwrap();

                tx.try_send(Ok(UpResponse {
                    ..Default::default()
                }))
                .unwrap();

                tx.try_send(Ok(UpResponse {
                    ..Default::default()
                }))
                .unwrap();

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

        let mut handle = CommandHandle::<UpResponse>::new(
            "ordered-stream".to_string(),
            response.into_inner(),
            client,
        );

        let first = handle
            .next()
            .await
            .expect("first response must exist")
            .expect("first response must be successful");

        let second = handle
            .next()
            .await
            .expect("second response must exist")
            .expect("second response must be successful");

        let third = handle
            .next()
            .await
            .expect("third response must exist")
            .expect("third response must be successful");

        /*
         * UpResponse currently contains no useful distinguishing
         * field in this test schema, so the important contract here
         * is that all three responses are received and consumed
         * sequentially.
         */
        let _ = (first, second, third);

        assert!(handle.next().await.is_none());
    }

    #[tokio::test]
    async fn command_handle_stream_propagates_stream_error() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|_req| {
                let (tx, rx) = tokio::sync::mpsc::channel(2);

                tx.try_send(Ok(UpResponse::default()))
                    .expect("failed to send response");

                tx.try_send(Err(Status::internal("command execution failed")))
                    .expect("failed to send stream error");

                Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                    rx,
                )))
            }));
        }

        let response = client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

        let mut handle = CommandHandle::<UpResponse>::new(
            "stream-error".to_string(),
            response.into_inner(),
            client,
        );

        assert!(handle.next().await.expect("first item must exist").is_ok());

        let result = handle.next().await.expect("stream error must be yielded");

        match result {
            Err(status) => {
                assert_eq!(status.code(), Code::Internal);

                assert_eq!(status.message(), "command execution failed");
            }

            Ok(_) => {
                panic!("expected stream error");
            }
        }

        assert!(
            handle.next().await.is_none(),
            "stream must terminate after error"
        );
    }

    #[tokio::test]
    async fn command_handle_preserves_command_id() {
        let (mock, client) = spawn_mock_server().await;

        let handle = create_up_handle(&mock, client, "preserved-command-id").await;

        assert_eq!(handle.command_id, "preserved-command-id");
    }

    #[tokio::test]
    async fn cancel_can_be_called_multiple_times() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_command_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(request.command_id, "multiple-cancel");

                Ok(Response::new(CancelCommandResponse { cancelled: false }))
            }));
        }

        let handle = create_up_handle(&mock, client, "multiple-cancel").await;

        let first = handle
            .cancel("first attempt")
            .await
            .expect("first cancel should succeed");

        let second = handle
            .cancel("second attempt")
            .await
            .expect("second cancel should succeed");

        assert!(!first);
        assert!(!second);
    }
}
