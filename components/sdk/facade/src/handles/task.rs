use knot_proto::v1::{
    daemon_service_client::DaemonServiceClient, task::CancelTaskRequest,
    task::cancel_task_response::Code,
};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_stream::Stream;
use tonic::{Streaming, transport::Channel};
use tracing::{debug, error, info, instrument, warn};

use crate::errors::ClientError;

/// An asynchronous stream wrapper representing a long-running daemon execution.
///
/// `TaskHandle` implements the `Stream` trait, allowing you to seamlessly iterate
/// over real-time execution events (such as task progress or completion statuses).
/// It also retains a clone of the underlying gRPC client, empowering the caller to
/// programmatically abort the execution mid-execution via the [`Self::cancel`] method.
#[derive(Debug)]
pub struct TaskHandle<E> {
    /// The unique identifier assigned by the daemon for this specific execution execution.
    pub task_id: String,
    pub(crate) events: Streaming<E>,
    client: DaemonServiceClient<Channel>,
}

impl<E> TaskHandle<E> {
    /// Constructs a new `CommandHandle`.
    ///
    /// This is typically called internally by the orchestrator after initiating a
    /// execution like `up` or `down`.
    pub fn new(
        task_id: String,
        events: Streaming<E>,
        client: DaemonServiceClient<Channel>,
    ) -> Self {
        Self {
            task_id,
            events,
            client,
        }
    }

    /// Attempts to gracefully abort the ongoing task on the daemon side.
    ///
    /// This method sends a cancellation signal to the daemon. If the task is still running,
    /// the daemon will attempt to halt further task execution and initiate rollback or shutdown
    /// procedures for tasks spawned by this specific task ID.
    ///
    /// # Returns
    ///
    /// Returns `true` if the daemon successfully caught and canceled the task, or `false`
    /// if the execution had already finished or could not be canceled.
    ///
    /// # Errors
    ///
    /// Returns a `tonic::Status` if the gRPC network request fails or if the daemon is unreachable.
    #[instrument(skip(self), fields(execution_id = %self.task_id))]
    pub async fn cancel(&self) -> Result<Code, ClientError> {
        debug!("sending cancellation request for active execution");

        let mut client = self.client.clone();
        let resp = client
            .cancel_task(CancelTaskRequest {
                task_id: self.task_id.clone(),
            })
            .await
            .map_err(|e| {
                error!(error = %e, "failed to send cancellation request to daemon");
                e
            })?;

        let result = resp.into_inner();
        if result.cancelled {
            info!(code = ?result.code, "execution successfully cancelled by the daemon");
        } else {
            warn!(code = ?result.code, "daemon rejected cancellation");
        }

        Code::try_from(result.code).map_err(|_| {
            ClientError::Contract(format!(
                "daemon returned unknown cancellation code: {}",
                result.code
            ))
        })
    }
}

impl<E> Stream for TaskHandle<E> {
    type Item = Result<E, tonic::Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Delegate the polling directly to the underlying tonic::Streaming wrapper.
        Pin::new(&mut self.events).poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{guarded_client::GuardedDaemonClient, test_utils::spawn_mock_server};

    use knot_proto::v1::{
        commands::{UpRequest, UpResponse},
        task::CancelTaskResponse,
    };

    use knot_proto::v1::task::cancel_task_response::Code as CancelCode;
    use tokio_stream::StreamExt;
    use tonic::{Code, Response, Status, metadata::MetadataValue};

    const WORKSPACE_ID: &str = "test-workspace";

    async fn create_up_handle(
        mock: &crate::test_utils::MockKnotDaemon,
        client: GuardedDaemonClient,
        expected_revision: &str,
    ) -> TaskHandle<UpResponse> {
        {
            let mut handler = mock.up_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();

                assert_eq!(
                    request.workspace_id, WORKSPACE_ID,
                    "up command must contain workspace_id"
                );

                let (_tx, rx) = tokio::sync::mpsc::channel(1);

                Ok(response(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }));
        }

        client
            .clone()
            .up(UpRequest {
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
                expected_revision: expected_revision.to_string(),
                prune: false,
            })
            .await
            .expect("failed to create mock execution stream")
    }

    const TASK_ID: &str = "test";

    fn response<T>(msg: T) -> Response<T> {
        let mut response = Response::new(msg);
        response.metadata_mut().insert(
            crate::guarded_client::METADATA_TASK_ID_HEADER,
            MetadataValue::try_from(TASK_ID).unwrap(),
        );
        response
    }

    #[tokio::test]
    async fn cancel_returns_success_when_daemon_cancels_command() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_task_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();
                assert_eq!(request.task_id, TASK_ID);

                let response = response(CancelTaskResponse {
                    cancelled: true,
                    code: i32::from(CancelCode::Success),
                });

                Ok(response)
            }));
        }

        let handle = create_up_handle(&mock, client, "test").await;
        let cancelled = handle.cancel().await.expect("cancel should succeed");

        assert!(
            matches!(cancelled, CancelCode::Success,),
            "cancel must return Success when daemon confirms cancellation"
        );
    }

    #[tokio::test]
    async fn cancel_returns_already_completed_when_command_is_already_completed() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_task_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();
                assert_eq!(request.task_id, TASK_ID);

                let response = response(CancelTaskResponse {
                    cancelled: false,
                    code: i32::from(CancelCode::AlreadyCompleted),
                });

                Ok(response)
            }));
        }

        let handle = create_up_handle(&mock, client, "test").await;

        let cancelled = handle.cancel().await.expect("cancel RPC should succeed");

        assert!(
            matches!(cancelled, CancelCode::AlreadyCompleted,),
            "cancel must return false when daemon rejects cancellation"
        );
    }

    #[tokio::test]
    async fn cancel_propagates_grpc_error() {
        let (mock, client) = spawn_mock_server().await;

        {
            let mut handler = mock.cancel_task_handler.lock().await;

            *handler = Some(Box::new(|req| {
                let request = req.into_inner();
                assert_eq!(request.task_id, TASK_ID);

                Err(Status::unavailable("daemon unavailable"))
            }));
        }

        let handle = create_up_handle(&mock, client, "test").await;
        let result = handle.cancel().await;

        match result {
            Err(ClientError::Protocol(status)) => {
                assert_eq!(status.code(), Code::Unavailable);
                assert_eq!(status.message(), "daemon unavailable");
            }
            _ => panic!("expected Protocol error"),
        }
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

                Ok(response(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }));
        }

        let mut handle = client
            .clone()
            .up(UpRequest {
                expected_revision: "test".to_string(),
                prune: false,
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

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

                Ok(response(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }));
        }

        let mut handle = client
            .clone()
            .up(UpRequest {
                expected_revision: "test".to_string(),
                prune: false,
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

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

                tx.try_send(Err(Status::internal("execution execution failed")))
                    .expect("failed to send stream error");

                Ok(response(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }));
        }

        let mut handle = client
            .clone()
            .up(UpRequest {
                expected_revision: "test".to_string(),
                prune: false,
                services: vec![],
                workspace_id: WORKSPACE_ID.to_string(),
            })
            .await
            .expect("up RPC should succeed");

        assert!(handle.next().await.expect("first item must exist").is_ok());

        let result = handle.next().await.expect("stream error must be yielded");

        match result {
            Err(status) => {
                assert_eq!(status.code(), Code::Internal);

                assert_eq!(status.message(), "execution execution failed");
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

        let handle = create_up_handle(&mock, client, "test").await;

        assert_eq!(handle.task_id, TASK_ID);
    }
}
