//! Middleware system for the Knot transport layer.
//!
//! This module provides a flexible, asynchronous pipeline for intercepting and
//! processing messages. It is built around the **Chain of Responsibility** pattern,
//! allowing developers to wrap the core transport logic with cross-cutting
//! concerns such as:
//!
//! - **Observability**: Logging, distributed tracing, and metrics collection.
//! - **Security**: Authentication, authorization, and IP filtering.
//! - **Reliability**: Rate limiting, retries, and circuit breaking.
//! - **Transformation**: Transparent encryption, compression, or data validation.
//!
//! ### Architecture Overview
//!
//! The middleware system consists of three primary components:
//!
//! 1. **[`Middleware`] Trait**: The interface that all custom layers must implement.
//! 2. **[`Pipeline`]**: An internal manager that holds a sequence of middlewares
//!    and coordinates their execution.
//! 3. **[`Next`]**: A transient object passed to each middleware, representing
//!    the "rest of the chain".
//!
//!
//!
//! ### Execution Order
//!
//! Middlewares are executed in the order they were added to the transport via
//! `add_middleware`. Each middleware has total control over the execution flow:
//! it can perform work **before** the next layer, **after** the next layer,
//! or **short-circuit** the entire process by not calling `next.run()`.
//!
//! ### Thread Safety and Lifetimes
//!
//! Since the Knot daemon is highly concurrent, all middlewares must be:
//! - `Send + Sync`: Safe to share and move between threads.
//! - `'static`: Living for the entire duration of the program.
//!
//! Messages are passed via a reference to [`MessageContext`], ensuring that
//! middlewares do not unnecessarily clone large payloads while still being able
//! to send replies or emit events.
//!
//! ### Example: A Simple Monitor Middleware
//!
//! ```rust,ignore
//! use async_trait::async_trait;
//!
//! #[derive(Debug)]
//! struct Monitor;
//!
//! #[async_trait]
//! impl<R, S> Middleware<R, S> for Monitor
//! where R: RawTransport, S: TransportSpec {
//!     async fn handle(&self, ctx: &MessageContext<'_, R, S>, next: Next<'_, R, S>) -> Result<(), TransportError> {
//!         let start = std::time::Instant::now();
//!         
//!         // Pass control to the next middleware or final handler
//!         let result = next.run(ctx).await;
//!         
//!         let duration = start.elapsed();
//!         println!("Message processed in {:?}", duration);
//!         
//!         result
//!     }
//! }
//! ```

use crate::{
    messages::MessageContext,
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

    /// Starts the execution of the middleware chain for a given context.
    ///
    /// This is the entry point for message processing. It begins by invoking
    /// the first middleware at index `0`.
    ///
    /// # Errors
    /// Returns a [`TransportError`] if any middleware in the chain fails.
    pub async fn execute(&self, ctx: &MessageContext<'_, R, S>) -> Result<(), TransportError> {
        self.invoke(0, ctx).await
    }

    /// Internal recursive function to trigger the middleware at the specified index.
    ///
    /// If no middleware is found at the given index, the chain is considered
    /// successfully completed, and it returns `Ok(())`.
    async fn invoke(
        &self,
        index: usize,
        ctx: &MessageContext<'_, R, S>,
    ) -> Result<(), TransportError> {
        if let Some(mw) = self.middlewares.get(index) {
            mw.handle(
                ctx,
                Next {
                    pipeline: self,
                    next_index: index + 1,
                },
            )
            .await?
        }
        Ok(())
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

/// A handle passed to middlewares to trigger the next layer in the pipeline.
///
/// `Next` acts as a "continuation" or a pointer to the remaining part of the
/// execution chain. It prevents middlewares from needing to know about the
/// structure of the [`Pipeline`] or their current position within it.
///
/// # Lifetimes
/// The `'a` lifetime ensures that the `Next` handle does not outlive
/// the [`Pipeline`] it references.
pub struct Next<'a, R: RawTransport, S: TransportSpec> {
    /// Reference to the parent pipeline.
    pipeline: &'a Pipeline<R, S>,
    /// The index of the middleware to be executed next.
    next_index: usize,
}

impl<'a, R, S> Next<'a, R, S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// Invokes the next middleware in the chain.
    ///
    /// This method should be called within a middleware's `handle` implementation
    /// to pass control further down the pipeline.
    ///
    /// # Important
    /// Failure to call `run` will result in "short-circuiting" the pipeline,
    /// meaning subsequent middlewares and the final message processing
    /// will be skipped.
    ///
    /// # Errors
    /// Propagates any [`TransportError`] encountered by downstream layers.
    pub async fn run(self, ctx: &MessageContext<'_, R, S>) -> Result<(), TransportError> {
        self.pipeline.invoke(self.next_index, ctx).await
    }
}

#[cfg(test)]
mod pipeline_tests {
    use crate::codec::JsonCodec;
    use crate::messages::{Message, MessageContext};
    use crate::middleware::{Next, Pipeline};
    use crate::transport::{MessageTransport, RawTransport, TransportSpec};
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

    impl MockRaw {
        pub fn new() -> (Self, mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
            let (in_tx, in_rx) = mpsc::channel(32);
            let (out_tx, out_rx) = mpsc::channel(32);

            let mock = Self {
                incoming_rx: Arc::new(Mutex::new(in_rx)),
                outgoing_tx: out_tx,
            };

            (mock, in_tx, out_rx)
        }
    }

    #[async_trait]
    impl RawTransport for MockRaw {
        async fn send_frame<'a>(&self, frame: &'a [u8]) -> Result<(), TransportError> {
            self.outgoing_tx.send(frame.to_vec()).await.ok();
            Ok(())
        }

        async fn recv_frame(&self) -> Result<Vec<u8>, TransportError> {
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

    type Ctx<'a> = MessageContext<'a, MockRaw, MockSpec>;
    type Transport = MessageTransport<MockRaw, MockSpec>;
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
        async fn handle(
            &self,
            ctx: &Ctx<'_>,
            next: Next<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.log.lock().await.push(format!("before:{}", self.name));
            next.run(ctx).await?;
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
        async fn handle(
            &self,
            _ctx: &Ctx<'_>,
            _next: Next<'_, MockRaw, MockSpec>,
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
        async fn handle(
            &self,
            _ctx: &Ctx<'_>,
            _next: Next<'_, MockRaw, MockSpec>,
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
        async fn handle(
            &self,
            ctx: &Ctx<'_>,
            next: Next<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            next.run(ctx).await?;
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
        async fn handle(
            &self,
            ctx: &Ctx<'_>,
            next: Next<'_, MockRaw, MockSpec>,
        ) -> Result<(), TransportError> {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next.run(ctx).await
        }
    }

    fn init_ctx<'a>(transport: &'a Transport) -> (Pipeline<MockRaw, MockSpec>, Ctx<'a>) {
        let message = TestMessage::request(0, TestReq::Ping);
        let pipeline = Pipeline::default();
        let ctx = MessageContext::new(message, transport);
        (pipeline, ctx)
    }

    fn init_transport() -> Transport {
        let (raw, _, _) = MockRaw::new();
        raw.to_messaged::<MockSpec>()
    }

    #[tokio::test]
    async fn test_empty_pipeline_returns_ok() {
        let transport = init_transport();
        let (pipeline, ctx) = init_ctx(&transport);

        let result = pipeline.execute(&ctx).await;
        assert!(result.is_ok(), "empty pipeline must return Ok(())");
    }

    #[tokio::test]
    async fn test_single_middleware_executes() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);

        let log = new_log();
        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "after:A"]);
    }

    #[tokio::test]
    async fn test_two_middleware_correct_order() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });
        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["before:A", "before:B", "after:B", "after:A"]);
    }

    #[tokio::test]
    async fn test_three_middleware_onion_order() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        for name in ["A", "B", "C"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();
        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec![
                "before:A", "before:B", "before:C", "after:C", "after:B", "after:A"
            ]
        );
    }

    #[tokio::test]
    async fn test_execute_idempotent() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "X".into(),
            log: log.clone(),
        });

        pipeline.execute(&ctx).await.unwrap();
        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events.len(), 4);
        assert_eq!(&events[0..2], &["before:X", "after:X"]);
        assert_eq!(&events[2..4], &["before:X", "after:X"]);
    }

    #[tokio::test]
    async fn test_single_middleware_error_propagates() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute(&ctx).await;

        assert!(result.is_err());
        matches!(result.unwrap_err(), TransportError::ConnectionClosed);
        assert_eq!(log.lock().await.clone(), vec!["error-mw"]);
    }

    #[tokio::test]
    async fn test_error_stops_chain_at_first_middleware() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(ErrorMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "B".into(),
            log: log.clone(),
        });

        let result = pipeline.execute(&ctx).await;

        assert!(result.is_err());
        let events = log.lock().await.clone();
        assert!(
            !events.contains(&"before:B".to_string()),
            "second middleware must not run after error"
        );
    }

    #[tokio::test]
    async fn test_error_in_second_middleware_propagates_to_first() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "A".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute(&ctx).await;

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
    async fn test_error_in_middle_of_three() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
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

        let result = pipeline.execute(&ctx).await;

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
    async fn test_error_type_preserved_through_chain() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Wrap".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let err = pipeline.execute(&ctx).await.unwrap_err();
        matches!(err, TransportError::ConnectionClosed);
    }

    #[tokio::test]
    async fn test_middleware_can_recover_error() {
        #[derive(Debug)]
        struct RecoverMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for RecoverMw {
            async fn handle(
                &self,
                ctx: &Ctx<'_>,
                next: Next<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                // swallow any inner error
                let _ = next.run(ctx).await;
                Ok(())
            }
        }

        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(RecoverMw);
        pipeline.add_middleware(ErrorMw { log: log.clone() });

        let result = pipeline.execute(&ctx).await;
        assert!(
            result.is_ok(),
            "RecoverMw swallowed the error; execute must be Ok"
        );
    }

    #[tokio::test]
    async fn test_terminating_middleware_blocks_rest() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(TerminateMw { log: log.clone() });
        pipeline.add_middleware(LogMw {
            name: "Never".into(),
            log: log.clone(),
        });

        let result = pipeline.execute(&ctx).await;

        assert!(result.is_ok());
        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate"]);
        assert!(!events.iter().any(|e| e.contains("Never")));
    }

    #[tokio::test]
    async fn test_terminating_middleware_in_middle() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
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

        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:A", "terminate", "after:A"],
            "C must never run; A's after-block must still run"
        );
    }

    #[tokio::test]
    async fn test_passthrough_before_terminate() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(PassthroughMw {
            name: "P".into(),
            log: log.clone(),
        });
        pipeline.add_middleware(TerminateMw { log: log.clone() });

        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["terminate", "pass:P"]);
    }

    #[tokio::test]
    async fn test_five_layer_onion_order() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        for name in ["1", "2", "3", "4", "5"] {
            pipeline.add_middleware(LogMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();

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
    async fn test_passthrough_chain_reverse_order() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        for name in ["X", "Y", "Z"] {
            pipeline.add_middleware(PassthroughMw {
                name: name.into(),
                log: log.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["pass:Z", "pass:Y", "pass:X"]);
    }

    #[tokio::test]
    async fn test_mixed_log_pass_log() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
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

        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(
            events,
            vec!["before:L1", "before:L2", "after:L2", "pass:P", "after:L1"]
        );
    }

    #[tokio::test]
    async fn test_each_middleware_called_exactly_once() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..10 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            10,
            "each of 10 middleware must be called exactly once"
        );
    }

    #[tokio::test]
    async fn test_counter_stops_at_error() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
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

        let _ = pipeline.execute(&ctx).await;

        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "only two CounterMw before the ErrorMw must have run"
        );
    }

    #[tokio::test]
    async fn test_last_middleware_no_next_call_is_fine() {
        #[derive(Debug)]
        struct FinalMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for FinalMw {
            async fn handle(
                &self,
                _ctx: &Ctx<'_>,
                _next: Next<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                Ok(())
            }
        }

        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);

        pipeline.add_middleware(FinalMw);

        assert!(pipeline.execute(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_next_run_at_end_of_chain_returns_ok() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();

        pipeline.add_middleware(LogMw {
            name: "Last".into(),
            log: log.clone(),
        });

        let result = pipeline.execute(&ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_next_run_forwards_ok() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        pipeline.add_middleware(CounterMw {
            counter: counter.clone(),
        });

        let result = pipeline.execute(&ctx).await;
        assert!(result.is_ok());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_add_middleware_in_loop() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..50 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 50);
    }

    #[tokio::test]
    async fn test_stress_100_layers() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..100 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 100);
    }

    #[tokio::test]
    async fn test_stress_1000_passthrough_layers() {
        let transport = init_transport();
        let (mut pipeline, ctx) = init_ctx(&transport);
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for _ in 0..1000 {
            pipeline.add_middleware(CounterMw {
                counter: counter.clone(),
            });
        }

        pipeline.execute(&ctx).await.unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1000);
    }

    #[tokio::test]
    async fn test_new_pipeline_is_debug() {
        let transport = init_transport();
        let (pipeline, _) = init_ctx(&transport);
        let _ = format!("{:?}", pipeline);
    }

    #[tokio::test]
    async fn test_conditional_short_circuit() {
        #[derive(Debug)]
        struct ConditionalMw {
            should_pass: bool,
            log: Log,
        }

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, MockSpec> for ConditionalMw {
            async fn handle(
                &self,
                ctx: &Ctx<'_>,
                next: Next<'_, MockRaw, MockSpec>,
            ) -> Result<(), TransportError> {
                if self.should_pass {
                    next.run(ctx).await?;
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

        let transport = init_transport();

        let (mut pipeline, ctx) = init_ctx(&transport);
        let log = new_log();
        pipeline.add_middleware(ConditionalMw {
            should_pass: false,
            log: log.clone(),
        });
        pipeline.add_middleware(LogMw {
            name: "N".into(),
            log: log.clone(),
        });
        pipeline.execute(&ctx).await.unwrap();

        let events = log.lock().await.clone();
        assert_eq!(events, vec!["conditional:blocked"]);

        let log2 = new_log();
        let (mut pipeline2, ctx2) = init_ctx(&transport);
        pipeline2.add_middleware(ConditionalMw {
            should_pass: true,
            log: log2.clone(),
        });
        pipeline2.add_middleware(LogMw {
            name: "N".into(),
            log: log2.clone(),
        });
        pipeline2.execute(&ctx2).await.unwrap();

        let events2 = log2.lock().await.clone();
        assert!(events2.contains(&"before:N".to_string()));
        assert!(events2.contains(&"conditional:passed".to_string()));
    }
}
