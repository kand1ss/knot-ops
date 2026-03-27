//! Middleware system for the Knot transport layer.
//!
//! This module provides a flexible, asynchronous pipeline for intercepting and
//! processing messages. It is built around a recursive **Chain of Responsibility** //! pattern, allowing developers to wrap the core transport logic with
//! cross-cutting concerns.
//!
//! ### Common Use Cases
//! - **Observability**: Logging, distributed tracing, and metrics collection.
//! - **Security**: Authentication, authorization, and IP filtering.
//! - **Reliability**: Rate limiting, retries, and circuit breaking.
//! - **Transformation**: Transparent encryption, compression, or metadata injection.
//!
//! ### Architecture Overview
//!
//! The middleware system consists of three primary components:
//!
//! 1. **[`Middleware`] Trait**: The core interface defining `on_recv` and `on_send` hooks.
//! 2. **[`Pipeline`]**: A thread-safe manager that holds an ordered sequence of
//!    middlewares in an `Arc<Vec<Box<dyn Middleware>>>`.
//! 3. **[`Inbound`] / [`Outbound`]**: Transient handles passed to each middleware,
//!    representing the "rest of the chain" (continuations).
//!
//! ### Execution Order and Interception
//!
//! Middlewares are executed in **FIFO** order (the order they were added). Each layer
//! has total control over the message journey:
//! - **Pre-processing**: Perform work before calling `next.run()`.
//! - **Post-processing**: Perform work after `next.run().await` returns.
//! - **Short-circuiting**: Stop the entire chain by **not** calling `next.run()`,
//!   effectively dropping or blocking the message.
//!
//! ### Thread Safety and Shared State
//!
//! Since the Knot daemon handles high concurrency via Tokio, all middlewares must be:
//! - **`Send + Sync`**: Safe to be shared and moved between threads.
//! - **`'static`**: Owned by the pipeline for the duration of the transport's life.
//!
//! To maintain state (like metrics) across requests, it is recommended to use
//! **Atomic** types or wrap shared data in an `Arc` before passing it to the
//! middleware constructor.
//!
//! ### Example: A Simple Logging Middleware
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//!
//! #[derive(Debug)]
//! struct Logger;
//!
//! #[async_trait]
//! impl<R, S> Middleware<R, S> for Logger
//! where
//!     R: RawTransport,
//!     S: TransportSpec
//! {
//!     async fn on_recv(
//!         &self,
//!         msg: &Message<S>,
//!         next: Inbound<'_, R, S>
//!     ) -> Result<(), TransportError> {
//!         println!("Inbound message: {:?}", msg.id);
//!         next.run(msg).await
//!     }
//!
//!     async fn on_send(
//!         &self,
//!         msg: &mut Message<S>,
//!         next: Outbound<'_, R, S>
//!     ) -> Result<(), TransportError> {
//!         msg.set_meta("processed-by", "logger-mw");
//!         next.run(msg).await
//!     }
//! }
//! ```

use crate::{
    messages::Message,
    middleware::traits::Middleware,
    transport::{RawTransport, TransportSpec},
};
use knot_core::errors::TransportError;

/// Defines the core abstractions for the Knot middleware system.
///
/// This module contains the [`Middleware`] trait, which is the fundamental
/// building block for extending the transport's behavior. By implementing
/// this trait, developers can hook into the message processing lifecycle
/// to provide cross-cutting concerns like logging, validation, or security.
pub mod traits;

/// A collection of middlewares organized into an executable chain.
///
/// The `Pipeline` is responsible for managing the registration of [`Middleware`]
/// implementations and orchestrating their execution in a specific order.
/// It uses an index-based recursion to move messages from one layer to the next.
///
/// # Concurrency
/// Since `Pipeline` is often wrapped in an `Arc` or protected by a lock (like `RwLock`)
/// within the transport, it is designed to be `Send + Sync`.
#[derive(Debug)]
pub struct Pipeline<R: RawTransport, S: TransportSpec> {
    /// Internal storage for boxed middleware trait objects.
    middlewares: Vec<Box<dyn Middleware<R, S>>>,
}

impl<R, S> Pipeline<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// Adds a middleware to the end of the pipeline.
    ///
    /// Middlewares are executed in the order they are added (FIFO).
    pub fn add_middleware<M: Middleware<R, S>>(&mut self, middleware: M) {
        self.middlewares.push(Box::new(middleware));
    }

    /// Initiates the inbound middleware processing for a received message.
    ///
    /// This is the primary entry point for processing data coming from the transport.
    /// It starts the recursive execution from the first middleware (index 0).
    ///
    /// # Errors
    /// Returns a [`TransportError`] if any middleware in the chain fails or
    /// explicitly blocks the message.
    pub async fn execute_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        self.invoke_recv(0, msg).await
    }

    /// Initiates the outbound middleware processing for a message being sent.
    ///
    /// This method ensures that all outgoing data is intercepted by the middleware
    /// chain (e.g., for adding metadata or encryption) before reaching the transport.
    ///
    /// # Errors
    /// Returns a [`TransportError`] if the chain execution is interrupted.
    pub async fn execute_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        self.invoke_send(0, msg).await
    }

    /// Creates a state object for the next middleware in the chain.
    ///
    /// This internal helper tracks the current position in the `middlewares` vector
    /// to ensure the message progresses linearly.
    async fn update_state(&self, index: usize) -> NextState<'_, R, S> {
        NextState {
            pipeline: self,
            next_index: index + 1,
        }
    }

    /// Recursively invokes the inbound middleware at the specified index.
    ///
    /// If the index is out of bounds, it signifies the end of the pipeline,
    /// and the processing is considered successful (`Ok(())`).
    ///
    /// The current middleware receives an [`Inbound`] handle, which it must call
    /// to continue the chain.
    async fn invoke_recv(
        &self,
        index: usize,
        msg: &Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        let Some(mw) = self.middlewares.get(index) else {
            return Ok(());
        };

        let mut state = self.update_state(index).await;
        mw.on_recv(msg, Inbound(&mut state)).await
    }

    /// Recursively invokes the outbound middleware at the specified index.
    ///
    /// Similar to `invoke_recv`, but operates on outgoing messages using the
    /// [`Outbound`] handle.
    async fn invoke_send(
        &self,
        index: usize,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
    ) -> Result<(), TransportError> {
        let Some(mw) = self.middlewares.get(index) else {
            return Ok(());
        };

        let mut state = self.update_state(index).await;
        mw.on_send(msg, Outbound(&mut state)).await
    }
}
impl<R, S> Default for Pipeline<R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    fn default() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }
}

/// A handle representing the remaining part of the inbound middleware chain.
///
/// Passed to [`Middleware::on_recv`], it allows the current layer to delegate
/// processing to the next middleware in the pipeline.
pub struct Inbound<'a, R: RawTransport, S: TransportSpec>(&'a mut NextState<'a, R, S>);

/// A handle representing the remaining part of the outbound middleware chain.
///
/// Passed to [`Middleware::on_send`], it serves as a continuation for outgoing
/// messages, moving them further down the stack towards the transport.
pub struct Outbound<'a, R: RawTransport, S: TransportSpec>(&'a mut NextState<'a, R, S>);

/// An internal "cursor" that tracks the progression of a message through the [`Pipeline`].
///
/// This structure maintains the current position within the middleware vector
/// and provides a reference back to the pipeline itself. It is designed to be
/// short-lived, existing only for the duration of a single message's journey.
///
/// # Lifetimes
/// * `'a`: Ties the state to the parent [`Pipeline`] to prevent dangling references.
struct NextState<'a, R: RawTransport, S: TransportSpec> {
    /// The index of the middleware to be executed next.
    next_index: usize,
    /// A reference to the parent pipeline containing the middleware vector.
    pipeline: &'a Pipeline<R, S>,
}

impl<'a, R, S> Inbound<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    pub async fn run(self, msg: &Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        let next_index = self.0.next_index;
        let pipeline = self.0.pipeline;

        if let Some(mw) = pipeline.middlewares.get(next_index) {
            self.0.next_index += 1;
            mw.on_recv(msg, self).await
        } else {
            Ok(())
        }
    }
}

impl<'a, R, S> Outbound<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    pub async fn run(self, msg: &mut Message<S::Req, S::Res, S::Ev>) -> Result<(), TransportError> {
        let next_index = self.0.next_index;
        let pipeline = self.0.pipeline;

        if let Some(mw) = pipeline.middlewares.get(next_index) {
            self.0.next_index += 1;
            mw.on_send(msg, self).await
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod pipeline_tests {
    use crate::codec::JsonCodec;
    use crate::messages::Message;
    use crate::middleware::{Inbound, Outbound, Pipeline};
    use crate::transport::{RawTransport, TransportSpec};
    use async_trait::async_trait;
    use knot_core::errors::TransportError;

    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc;

    #[derive(Debug, Clone)]
    pub struct MockRaw {
        pub incoming_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
        pub outgoing_tx: mpsc::Sender<Vec<u8>>,
    }

    #[async_trait]
    impl RawTransport for MockRaw {
        async fn send_frame_internal<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
            self.outgoing_tx.send(frame.to_vec()).await.ok();
            Ok(())
        }

        async fn recv_frame_internal(&self) -> Result<Vec<u8>, TransportError> {
            let mut rx = self.incoming_rx.lock().await;
            rx.recv().await.ok_or(TransportError::UnexpectedMessage)
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub enum TestReq {
        Ping,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub enum TestRes {
        Pong,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub enum TestEv {
        Event,
    }

    #[derive(Debug, Clone)]
    pub struct MockSpec;
    impl TransportSpec for MockSpec {
        type Req = TestReq;
        type Res = TestRes;
        type Ev = TestEv;
        type C = JsonCodec;
    }

    type Log = Arc<Mutex<Vec<String>>>;
    type TestMessage = Message<TestReq, TestRes, TestEv>;

    fn new_log() -> Log {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[derive(Debug)]
    struct LogMw {
        name: String,
        log: Log,
    }

    #[async_trait]
    impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for LogMw {
        async fn on_recv(
            &self,
            msg: &TestMessage,
            next: Inbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push(format!("before:{}", self.name));
            next.run(msg).await?;
            self.log.lock().await.push(format!("after:{}", self.name));
            Ok(())
        }

        async fn on_send(
            &self,
            msg: &mut TestMessage,
            next: Outbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push(format!("before:{}", self.name));
            next.run(msg).await?;
            self.log.lock().await.push(format!("after:{}", self.name));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TerminateMw {
        log: Log,
    }

    #[async_trait]
    impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for TerminateMw {
        async fn on_recv(
            &self,
            _msg: &TestMessage,
            _next: Inbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push("terminate".to_string());
            Ok(())
        }

        async fn on_send(
            &self,
            _msg: &mut TestMessage,
            _next: Outbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push("terminate".to_string());
            Ok(())
        }
    }

    #[derive(Debug)]
    struct ErrorMw {
        log: Log,
    }

    #[async_trait]
    impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for ErrorMw {
        async fn on_recv(
            &self,
            _msg: &TestMessage,
            _next: Inbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push("error-mw".to_string());
            Err(TransportError::ConnectionClosed)
        }

        async fn on_send(
            &self,
            _msg: &mut TestMessage,
            _next: Outbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push("error-mw".to_string());
            Err(TransportError::ConnectionClosed)
        }
    }

    #[derive(Debug)]
    struct PassthroughMw {
        name: String,
        log: Log,
    }

    #[async_trait]
    impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for PassthroughMw {
        async fn on_recv(
            &self,
            msg: &TestMessage,
            next: Inbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            next.run(msg).await?;
            self.log.lock().await.push(format!("pass:{}", self.name));
            Ok(())
        }

        async fn on_send(
            &self,
            msg: &mut TestMessage,
            next: Outbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            next.run(msg).await?;
            self.log.lock().await.push(format!("pass:{}", self.name));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct CounterMw {
        counter: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for CounterMw {
        async fn on_recv(
            &self,
            msg: &TestMessage,
            next: Inbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next.run(msg).await
        }

        async fn on_send(
            &self,
            msg: &mut TestMessage,
            next: Outbound<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next.run(msg).await
        }
    }

    fn init() -> (Pipeline<MockRaw, MockSpec>, TestMessage) {
        let message = TestMessage::request(0, TestReq::Ping);
        let pipeline = Pipeline::default();
        (pipeline, message)
    }

    #[tokio::test]
    async fn test_recv_empty_pipeline_returns_ok() {
        let (pipeline, ctx) = init();

        let result = pipeline.execute_recv(&ctx).await;
        assert!(result.is_ok(), "empty pipeline must return Ok(())");
    }

    #[tokio::test]
    async fn test_send_empty_pipeline_returns_ok() {
        let (pipeline, mut ctx) = init();

        let result = pipeline.execute_send(&mut ctx).await;
        assert!(result.is_ok(), "empty pipeline must return Ok(())");
    }

    #[tokio::test]
    async fn test_recv_single_middleware_executes() {
        let (mut pipeline, ctx) = init();

        let log = new_log();
        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "after:A"]);
    }

    #[tokio::test]
    async fn test_send_single_middleware_executes() {
        let (mut pipeline, mut ctx) = init();

        let log = new_log();
        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "after:A"]);
    }

    #[tokio::test]
    async fn test_recv_two_middleware_correct_order() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });
        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "before:B", "after:B", "after:A"]);
    }

    #[tokio::test]
    async fn test_send_two_middleware_correct_order() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });
        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "before:B", "after:B", "after:A"]);
    }

    #[tokio::test]
    async fn test_recv_three_middleware_onion_order() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        for name in ["A", "B", "C"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();
        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "before:A", "before:B", "before:C", "after:C", "after:B", "after:A"
            ]
        );
    }

    #[tokio::test]
    async fn test_send_three_middleware_onion_order() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        for name in ["A", "B", "C"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();
        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "before:A", "before:B", "before:C", "after:C", "after:B", "after:A"
            ]
        );
    }

    #[tokio::test]
    async fn test_recv_execute_idempotent() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "X".into(),
            log: log.clone(),
        });

        pipeline.execute_recv(&ctx).await.unwrap();
        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events.len(), 4);
        assert_eq!(&events[0..2], &["before:X", "after:X"]);
        assert_eq!(&events[2..4], &["before:X", "after:X"]);
    }

    #[tokio::test]
    async fn test_send_execute_idempotent() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "X".into(),
            log: log.clone(),
        });

        pipeline.execute_send(&mut ctx).await.unwrap();
        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events.len(), 4);
        assert_eq!(&events[0..2], &["before:X", "after:X"]);
        assert_eq!(&events[2..4], &["before:X", "after:X"]);
    }

    #[tokio::test]
    async fn test_recv_single_middleware_error_propagates() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_recv(&ctx).await;

        assert!(result.is_err());
        matches!(result.unwrap_err(), TransportError::ConnectionClosed);
        assert_eq!(log.lock().await.clone(), vec!["error-mw"]);
    }

    #[tokio::test]
    async fn test_send_single_middleware_error_propagates() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_send(&mut ctx).await;

        assert!(result.is_err());
        matches!(result.unwrap_err(), TransportError::ConnectionClosed);
        assert_eq!(log.lock().await.clone(), vec!["error-mw"]);
    }

    #[tokio::test]
    async fn test_recv_error_stops_chain_at_first_middleware() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_recv(&ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();
        assert!(
            !events.contains(&"before:B".to_string()),
            "second middleware must not run after error"
        );
    }

    #[tokio::test]
    async fn test_send_error_stops_chain_at_first_middleware() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_send(&mut ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();
        assert!(
            !events.contains(&"before:B".to_string()),
            "second middleware must not run after error"
        );
    }

    #[tokio::test]
    async fn test_recv_error_in_second_middleware_propagates_to_first() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_recv(&ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();

        assert!(events.contains(&"before:A".to_string()));
        assert!(events.contains(&"error-mw".to_string()));
        assert!(
            !events.contains(&"after:A".to_string()),
            "after-A must not run because next.run returned Err"
        );
    }

    #[tokio::test]
    async fn test_send_error_in_second_middleware_propagates_to_first() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_send(&mut ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();

        assert!(events.contains(&"before:A".to_string()));
        assert!(events.contains(&"error-mw".to_string()));
        assert!(
            !events.contains(&"after:A".to_string()),
            "after-A must not run because next.run returned Err"
        );
    }

    #[tokio::test]
    async fn test_recv_error_in_middle_of_three() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "C".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_recv(&ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();
        assert!(
            !events
                .iter()
                .any(|e| e.starts_with("before:C") || e.starts_with("after:C")),
            "C must be completely skipped"
        );
    }

    #[tokio::test]
    async fn test_send_error_in_middle_of_three() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "C".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_send(&mut ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();
        assert!(
            !events
                .iter()
                .any(|e| e.starts_with("before:C") || e.starts_with("after:C")),
            "C must be completely skipped"
        );
    }

    #[tokio::test]
    async fn test_recv_error_type_preserved_through_chain() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Wrap".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let err = pipeline.execute_recv(&ctx).await.unwrap_err();
        matches!(err, TransportError::ConnectionClosed);
    }

    #[tokio::test]
    async fn test_send_error_type_preserved_through_chain() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Wrap".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let err = pipeline.execute_send(&mut ctx).await.unwrap_err();
        matches!(err, TransportError::ConnectionClosed);
    }

    #[tokio::test]
    async fn test_recv_middleware_can_recover_error() {
        #[derive(Debug)]
        struct RecoverMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for RecoverMw {
            async fn on_recv(
                &self,
                msg: &TestMessage,
                next: Inbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                let _ = next.run(msg).await;
                Ok(())
            }

            async fn on_send(
                &self,
                msg: &mut TestMessage,
                next: Outbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                let _ = next.run(msg).await;
                Ok(())
            }
        }

        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(RecoverMw);
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_recv(&ctx).await;
        assert!(
            result.is_ok(),
            "RecoverMw swallowed the error; execute must be Ok"
        );
    }

    #[tokio::test]
    async fn test_send_middleware_can_recover_error() {
        #[derive(Debug)]
        struct RecoverMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for RecoverMw {
            async fn on_recv(
                &self,
                msg: &TestMessage,
                next: Inbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                let _ = next.run(msg).await;
                Ok(())
            }

            async fn on_send(
                &self,
                msg: &mut TestMessage,
                next: Outbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                let _ = next.run(msg).await;
                Ok(())
            }
        }

        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(RecoverMw);
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute_send(&mut ctx).await;
        assert!(
            result.is_ok(),
            "RecoverMw swallowed the error; execute must be Ok"
        );
    }

    #[tokio::test]
    async fn test_recv_terminating_middleware_blocks_rest() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(TerminateMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "Never".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_recv(&ctx).await;

        assert!(result.is_ok());
        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate"]);
        assert!(!events.iter().any(|e| e.contains("Never")));
    }

    #[tokio::test]
    async fn test_send_terminating_middleware_blocks_rest() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(TerminateMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "Never".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_send(&mut ctx).await;

        assert!(result.is_ok());
        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate"]);
        assert!(!events.iter().any(|e| e.contains("Never")));
    }

    #[tokio::test]
    async fn test_recv_terminating_middleware_in_middle() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(TerminateMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "C".into(),
            log: log.clone(),
        });

        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:A", "terminate", "after:A"],
            "C must never run; A's after-block must still run"
        );
    }

    #[tokio::test]
    async fn test_send_terminating_middleware_in_middle() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(TerminateMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "C".into(),
            log: log.clone(),
        });

        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:A", "terminate", "after:A"],
            "C must never run; A's after-block must still run"
        );
    }

    #[tokio::test]
    async fn test_recv_passthrough_before_terminate() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(PassthroughMw {
            name: "P".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(TerminateMw { log: log.clone() });

        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate", "pass:P"]);
    }

    #[tokio::test]
    async fn test_send_passthrough_before_terminate() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(PassthroughMw {
            name: "P".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(TerminateMw { log: log.clone() });

        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate", "pass:P"]);
    }

    #[tokio::test]
    async fn test_recv_five_layer_onion_order() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        for name in ["1", "2", "3", "4", "5"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "before:1", "before:2", "before:3", "before:4", "before:5", "after:5", "after:4",
                "after:3", "after:2", "after:1",
            ]
        );
    }

    #[tokio::test]
    async fn test_send_five_layer_onion_order() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        for name in ["1", "2", "3", "4", "5"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "before:1", "before:2", "before:3", "before:4", "before:5", "after:5", "after:4",
                "after:3", "after:2", "after:1",
            ]
        );
    }

    #[tokio::test]
    async fn test_recv_passthrough_chain_reverse_order() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        for name in ["X", "Y", "Z"] {
            pipeline.add_middleware(PassthroughMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["pass:Z", "pass:Y", "pass:X"]);
    }

    #[tokio::test]
    async fn test_send_passthrough_chain_reverse_order() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        for name in ["X", "Y", "Z"] {
            pipeline.add_middleware(PassthroughMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["pass:Z", "pass:Y", "pass:X"]);
    }

    #[tokio::test]
    async fn test_recv_mixed_log_pass_log() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "L1".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(PassthroughMw {
            name: "P".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "L2".into(),
            log: log.clone(),
        });

        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:L1", "before:L2", "after:L2", "pass:P", "after:L1"]
        );
    }

    #[tokio::test]
    async fn test_send_mixed_log_pass_log() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "L1".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(PassthroughMw {
            name: "P".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "L2".into(),
            log: log.clone(),
        });

        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:L1", "before:L2", "after:L2", "pass:P", "after:L1"]
        );
    }

    #[tokio::test]
    async fn test_recv_each_middleware_called_exactly_once() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..10 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            10,
            "each of 10 middleware must be called exactly once"
        );
    }

    #[tokio::test]
    async fn test_send_each_middleware_called_exactly_once() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..10 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            10,
            "each of 10 middleware must be called exactly once"
        );
    }

    #[tokio::test]
    async fn test_recv_counter_stops_at_error() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let log = new_log();

        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });
        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });
        pipeline.add_middleware(ErrorMw { log });
        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });

        let _ = pipeline.execute_recv(&ctx).await;

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "only two CounterMw before the ErrorMw must have run"
        );
    }

    #[tokio::test]
    async fn test_send_counter_stops_at_error() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let log = new_log();

        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });
        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });
        pipeline.add_middleware(ErrorMw { log });
        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });

        let _ = pipeline.execute_send(&mut ctx).await;

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "only two CounterMw before the ErrorMw must have run"
        );
    }

    #[tokio::test]
    async fn test_recv_last_middleware_no_next_call_is_fine() {
        #[derive(Debug)]
        struct FinalMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for FinalMw {
            async fn on_recv(
                &self,
                _msg: &TestMessage,
                _next: Inbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                Ok(())
            }
        }

        let (mut pipeline, ctx) = init();
        pipeline.add_middleware(FinalMw);

        assert!(pipeline.execute_recv(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_send_last_middleware_no_next_call_is_fine() {
        #[derive(Debug)]
        struct FinalMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for FinalMw {
            async fn on_recv(
                &self,
                _msg: &TestMessage,
                _next: Inbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                Ok(())
            }

            async fn on_send(
                &self,
                _msg: &mut TestMessage,
                _next: Outbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                Ok(())
            }
        }

        let (mut pipeline, mut ctx) = init();
        pipeline.add_middleware(FinalMw);

        assert!(pipeline.execute_send(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_recv_next_run_at_end_of_chain_returns_ok() {
        let (mut pipeline, ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Last".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_recv(&ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_next_run_at_end_of_chain_returns_ok() {
        let (mut pipeline, mut ctx) = init();
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Last".into(),
            log: log.clone(),
        });

        let result = pipeline.execute_send(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_recv_next_run_forwards_ok() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });

        let result = pipeline.execute_recv(&ctx).await;
        assert!(result.is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_send_next_run_forwards_ok() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });

        let result = pipeline.execute_send(&mut ctx).await;
        assert!(result.is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_recv_add_middleware_in_loop() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..50 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn test_send_add_middleware_in_loop() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..50 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn test_recv_stress_100_layers() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..100 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 100);
    }

    #[tokio::test]
    async fn test_send_stress_100_layers() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..100 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 100);
    }

    #[tokio::test]
    async fn test_recv_stress_1000_passthrough_layers() {
        let (mut pipeline, ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..1000 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_recv(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1000);
    }

    #[tokio::test]
    async fn test_send_stress_1000_passthrough_layers() {
        let (mut pipeline, mut ctx) = init();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..1000 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute_send(&mut ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1000);
    }

    #[tokio::test]
    async fn test_new_pipeline_is_debug() {
        let (pipeline, _ctx) = init();
        let _ = format!("{:?}", pipeline);
    }

    #[tokio::test]
    async fn test_recv_conditional_short_circuit() {
        #[derive(Debug)]
        struct ConditionalMw {
            should_pass: bool,
            log: Log,
        }

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for ConditionalMw {
            async fn on_recv(
                &self,
                msg: &TestMessage,
                next: Inbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                if self.should_pass {
                    next.run(msg).await?;
                    self.log.lock().await.push("conditional:passed".to_string());
                } else {
                    self.log
                        .lock()
                        .await
                        .push("conditional:blocked".to_string());
                }
                Ok(())
            }
        }

        let (mut pipeline, ctx) = init();
        let log = new_log();
        pipeline.add_middleware(ConditionalMw {
            should_pass: false,
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "N".into(),
            log: log.clone(),
        });
        pipeline.execute_recv(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["conditional:blocked"]);

        let log2 = new_log();
        let (mut pipeline2, ctx2) = init();
        pipeline2.add_middleware(ConditionalMw {
            should_pass: true,
            log: log2.clone(),
        });
        pipeline2.add_middleware(LogMw {
            name: "N".into(),
            log: log2.clone(),
        });
        pipeline2.execute_recv(&ctx2).await.unwrap();

        let events2 = log2.lock().await.clone();
        assert!(events2.contains(&"before:N".to_string()));
        assert!(events2.contains(&"conditional:passed".to_string()));
    }

    #[tokio::test]
    async fn test_send_conditional_short_circuit() {
        #[derive(Debug)]
        struct ConditionalMw {
            should_pass: bool,
            log: Log,
        }

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for ConditionalMw {
            async fn on_send(
                &self,
                msg: &mut TestMessage,
                next: Outbound<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                if self.should_pass {
                    next.run(msg).await?;
                    self.log.lock().await.push("conditional:passed".to_string());
                } else {
                    self.log
                        .lock()
                        .await
                        .push("conditional:blocked".to_string());
                }
                Ok(())
            }
        }

        let (mut pipeline, mut ctx) = init();
        let log = new_log();
        pipeline.add_middleware(ConditionalMw {
            should_pass: false,
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "N".into(),
            log: log.clone(),
        });
        pipeline.execute_send(&mut ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["conditional:blocked"]);

        let log2 = new_log();
        let (mut pipeline2, mut ctx2) = init();
        pipeline2.add_middleware(ConditionalMw {
            should_pass: true,
            log: log2.clone(),
        });
        pipeline2.add_middleware(LogMw {
            name: "N".into(),
            log: log2.clone(),
        });
        pipeline2.execute_send(&mut ctx2).await.unwrap();

        let events2 = log2.lock().await.clone();
        assert!(events2.contains(&"before:N".to_string()));
        assert!(events2.contains(&"conditional:passed".to_string()));
    }
}
