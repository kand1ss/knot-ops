//! Transport layer metrics and instrumentation.
//!
//! This module provides comprehensive metrics collection for message and request tracking
//! across the transport layer. It includes real-time statistics on message throughput,
//! request latency, and failure tracking with minimal synchronization overhead.
//!
//! # Architecture
//!
//! The module is organized around three main statistics types:
//! - **Messages**: High-level throughput metrics (sent, received, failed).
//! - **Requests**: Low-level request/response tracking with latency percentiles.
//! - **Events**: Event-specific throughput metrics.
//!
//! Statistics are collected through the [`MetricsMiddleware`], which hooks into
//! the inbound and outbound message pipeline.
//!
//! # Thread Safety
//!
//! All stats types use atomic operations (`AtomicU64`) for counters and a `RwLock`-protected
//! shared state for latency calculations, ensuring safe concurrent access without locks
//! on the hot path for simple increments.
use crate::{
    messages::{Message, MessageKind},
    middleware::{Inbound, Outbound, traits::Middleware},
    transport::{RawTransport, TransportSpec},
};
use async_trait::async_trait;
use knot_core::errors::TransportError;
use std::marker::PhantomData;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;
use tracing::{debug, instrument, trace, warn};

/// Snapshot of message-level statistics.
///
/// This struct holds immutable counts of messages processed at the transport layer.
/// It represents a point-in-time snapshot of metrics and is useful for reporting
/// or monitoring overall message health without concern for latency details.
///
/// # Fields
///
/// * `total_sent` - Total number of messages sent since the start of the session.
/// * `total_received` - Total number of messages received since the start of the session.
/// * `total_failed` - Total number of messages that failed to process.
#[derive(Debug)]
pub struct MessagesStatsData {
    pub total_sent: u64,
    pub total_received: u64,
    pub total_failed: u64,
}

/// Real-time message statistics collector.
///
/// This struct tracks message throughput using atomic counters. It is designed to be
/// embedded in middleware or background services with minimal contention. All operations
/// use relaxed atomic ordering except for the final `get()` snapshot, which uses
/// acquire semantics to ensure visibility of prior writes.
///
/// # Examples
///
/// ```rust,ignore
/// let stats = MessagesStats::default();
/// stats.send();      // Increment sent counter
/// stats.receive();   // Increment received counter
/// let data = stats.get();
/// println!("Sent: {}, Received: {}", data.total_sent, data.total_received);
/// ```
#[derive(Debug, Default)]
pub struct MessagesStats {
    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub total_failed: AtomicU64,
}
impl MessagesStats {
    /// Record that a message was sent.
    ///
    /// Increments the `total_sent` counter and logs the new total at trace level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "message_sent_metrics", level = "trace")]
    fn send(&self) {
        let n = self.total_sent.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_sent = n, "Message sent recorded");
    }

    /// Record that a message was received.
    ///
    /// Increments the `total_received` counter and logs the new total at trace level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "message_received_metrics", level = "trace")]
    fn receive(&self) {
        let n = self.total_received.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_received = n, "Message received recorded");
    }

    /// Record that a message failed.
    ///
    /// Increments the `total_failed` counter and logs the new total at warn level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "message_failed_metrics", level = "warn")]
    fn fail(&self) {
        let n = self.total_failed.fetch_add(1, Ordering::Relaxed) + 1;
        warn!(total_failed = n, "Message failed recorded");
    }

    /// Retrieve a snapshot of current message statistics.
    ///
    /// Returns a [`MessagesStatsData`] struct containing the current values of all counters.
    /// Uses acquire ordering to ensure visibility of all prior increments across threads.
    ///
    /// # Returns
    ///
    /// A [`MessagesStatsData`] snapshot containing current totals.
    pub fn get(&self) -> MessagesStatsData {
        MessagesStatsData {
            total_sent: self.total_sent.load(Ordering::Acquire),
            total_received: self.total_received.load(Ordering::Acquire),
            total_failed: self.total_failed.load(Ordering::Acquire),
        }
    }
}

use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;

/// Shared mutable state for request-level statistics.
///
/// This internal struct maintains detailed per-request tracking data:
/// - Active pending requests indexed by message ID.
/// - Failed requests that may be retried.
/// - Latency calculations (min, max, average).
///
/// It is protected by a [`RwLock`] to allow concurrent reads during statistics snapshots
/// and exclusive writes during latency updates.
#[derive(Debug, Default)]
struct RequestsSharedState {
    /// Map of in-flight requests: message ID -> timestamp when sent.
    pending_requests: HashMap<u32, Instant>,
    /// Set of request IDs that failed and may be retried.
    failed: HashSet<u32>,

    total_received: u64,
    avg_latency: u64,
    min_latency: u64,
    max_latency: u64,
}
impl RequestsSharedState {
    /// Create a new requests shared state with latency boundaries initialized.
    ///
    /// Sets `min_latency` to `u64::MAX` so that the first measurement properly
    /// updates it to the actual minimum.
    fn new() -> Self {
        Self {
            min_latency: u64::MAX,
            ..Default::default()
        }
    }
}

/// Snapshot of request-level statistics including latency metrics.
///
/// This struct captures a point-in-time view of request/response performance,
/// including throughput counts and latency percentiles (min, max, average).
/// Useful for monitoring request performance and diagnosing transport issues.
///
/// # Fields
///
/// * `total_sent` - Total number of requests sent.
/// * `total_received` - Total number of responses received.
/// * `total_failed` - Total number of requests that failed.
/// * `total_retried` - Total number of requests retried after failure.
/// * `min_latency` - Minimum observed latency in milliseconds.
/// * `max_latency` - Maximum observed latency in milliseconds.
/// * `avg_latency` - Average latency in milliseconds (computed via running mean).
#[derive(Debug)]
pub struct RequestsStatsData {
    pub total_sent: u64,
    pub total_received: u64,
    pub total_failed: u64,
    pub total_retried: u64,
    pub min_latency: u64,
    pub max_latency: u64,
    pub avg_latency: u64,
}

/// Real-time request/response statistics with latency tracking.
///
/// This struct tracks individual request lifecycle events (send, receive, fail) and
/// computes latency statistics on response arrival. Atomic counters handle throughput,
/// while a lock-protected shared state handles per-request timing data.
///
/// Request IDs are used to correlate sends with responses. If a request fails and is
/// retried, it is marked in the `failed` set and the retry is counted separately.
///
/// # Latency Calculation
///
/// Average latency is computed using Welford's online algorithm to avoid overflow:
/// ```text
/// new_avg = old_avg + (sample - old_avg) / count
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// let stats = RequestsStats::default();
/// stats.send(request_id).await;
/// // ... request is processed ...
/// stats.receive(request_id).await;  // Latency is recorded
/// let data = stats.get().await;
/// println!("Avg latency: {} ms", data.avg_latency);
/// ```
#[derive(Debug)]
pub struct RequestsStats {
    /// Lock-protected state for per-request tracking and latency computation.
    shared: RwLock<RequestsSharedState>,

    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub total_failed: AtomicU64,
    pub total_retried: AtomicU64,
}
impl Default for RequestsStats {
    fn default() -> Self {
        Self {
            shared: RwLock::new(RequestsSharedState::new()),
            total_sent: AtomicU64::new(0),
            total_received: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
            total_retried: AtomicU64::new(0),
        }
    }
}
impl RequestsStats {
    /// Record that a request was sent.
    ///
    /// Stores the send timestamp for the given message ID and increments the sent counter.
    /// If this request was previously marked as failed, increments the retry counter instead
    /// of logging a simple send.
    ///
    /// # Arguments
    ///
    /// * `msg_id` - Unique identifier for this request, used to correlate with responses.
    #[instrument(skip(self), name = "request_sent_metrics", level = "trace")]
    async fn send(&self, msg_id: u32) {
        let total = self.total_sent.fetch_add(1, Ordering::Relaxed) + 1;
        let mut shared = self.shared.write().await;

        shared.pending_requests.insert(msg_id, Instant::now());
        if shared.failed.remove(&msg_id) {
            let retried = self.total_retried.fetch_add(1, Ordering::Relaxed) + 1;
            debug!(
                total_retried = retried,
                "Request retried after previous failure"
            );
        } else {
            trace!(total_sent = total, "Request sent recorded");
        }
    }

    /// Record that a response was received for a request.
    ///
    /// Looks up the corresponding send timestamp, computes the request latency in milliseconds,
    /// and updates min/max/average latency statistics. If no pending request is found,
    /// logs a warning (duplicate or unsolicited response).
    ///
    /// # Arguments
    ///
    /// * `msg_id` - Unique identifier for the request that this response completes.
    #[instrument(skip(self), name = "request_received_metrics", level = "trace")]
    async fn receive(&self, msg_id: u32) {
        let mut shared = self.shared.write().await;

        if let Some(date) = shared.pending_requests.remove(&msg_id) {
            let ms = date.elapsed().as_millis() as u64;
            shared.total_received += 1;

            shared.min_latency = shared.min_latency.min(ms);
            shared.max_latency = shared.max_latency.max(ms);

            let avg = shared.avg_latency as i64;
            let new_avg = avg + (ms as i64 - avg) / shared.total_received as i64;
            shared.avg_latency = new_avg as u64;

            let total = self.total_received.fetch_add(1, Ordering::Relaxed) + 1;
            debug!(
                latency_ms = ms,
                avg_latency_ms = shared.avg_latency,
                min_latency_ms = shared.min_latency,
                max_latency_ms = shared.max_latency,
                total_received = total,
                "Request completed recorded"
            );
        } else {
            self.total_received.fetch_add(1, Ordering::Relaxed);
            warn!("Received response for unknown or duplicate request id");
        }
    }

    /// Record that a request failed.
    ///
    /// Removes the request from pending, increments the failed counter, and marks it
    /// in the failed set for potential retry tracking. If the request ID was not found
    /// in pending requests, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `msg_id` - Unique identifier for the request that failed.
    #[instrument(skip(self), name = "request_failed_metrics", level = "warn")]
    async fn fail(&self, msg_id: u32) {
        let mut shared = self.shared.write().await;

        if shared.pending_requests.remove(&msg_id).is_some() {
            let total = self.total_failed.fetch_add(1, Ordering::Relaxed) + 1;
            shared.failed.insert(msg_id);
            warn!(
                total_failed = total,
                "Request failed, queued for retry tracking"
            );
        } else {
            trace!("Fail called for id not in pending, ignoring...");
        }
    }

    /// Retrieve a snapshot of current request statistics.
    ///
    /// Returns a [`RequestsStatsData`] struct containing current counters and latency metrics.
    /// Acquires a read lock to safely access latency data.
    ///
    /// # Returns
    ///
    /// A [`RequestsStatsData`] snapshot containing current totals and latency statistics.
    pub async fn get(&self) -> RequestsStatsData {
        let shared = self.shared.read().await;

        RequestsStatsData {
            total_sent: self.total_sent.load(Ordering::Acquire),
            total_received: self.total_received.load(Ordering::Acquire),
            total_failed: self.total_failed.load(Ordering::Acquire),
            total_retried: self.total_retried.load(Ordering::Acquire),
            min_latency: shared.min_latency,
            max_latency: shared.max_latency,
            avg_latency: shared.avg_latency,
        }
    }
}

/// Snapshot of event-level statistics.
///
/// Similar to [`MessagesStatsData`], this struct holds immutable counts of events
/// processed at the transport layer. It is useful for monitoring event throughput
/// and failure rates independently from message and request metrics.
///
/// # Fields
///
/// * `total_sent` - Total number of events sent since the start of the session.
/// * `total_received` - Total number of events received since the start of the session.
/// * `total_failed` - Total number of events that failed to process.
#[derive(Debug)]
pub struct EventsStatsData {
    pub total_sent: u64,
    pub total_received: u64,
    pub total_failed: u64,
}

/// Real-time event statistics collector.
///
/// This struct tracks event throughput using atomic counters, similar to [`MessagesStats`].
/// It is designed for efficient concurrent access with minimal overhead.
///
/// # Examples
///
/// ```rust,ignore
/// let stats = EventsStats::default();
/// stats.send();      // Increment sent counter
/// stats.receive();   // Increment received counter
/// let data = stats.get();
/// println!("Events sent: {}", data.total_sent);
/// ```
#[derive(Debug, Default)]
pub struct EventsStats {
    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub total_failed: AtomicU64,
}
impl EventsStats {
    /// Record that an event was sent.
    ///
    /// Increments the `total_sent` counter and logs the new total at trace level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "event_sent_metrics", level = "trace")]
    fn send(&self) {
        let n = self.total_sent.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_sent = n, "Event sent recorded");
    }

    /// Record that an event was received.
    ///
    /// Increments the `total_received` counter and logs the new total at trace level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "event_received_metrics", level = "trace")]
    fn receive(&self) {
        let n = self.total_received.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_received = n, "Event received recorded");
    }

    /// Record that an event failed.
    ///
    /// Increments the `total_failed` counter and logs the new total at warn level.
    /// This operation is lock-free and uses relaxed atomic ordering.
    #[instrument(skip(self), name = "event_failed_metrics", level = "warn")]
    fn fail(&self) {
        let n = self.total_failed.fetch_add(1, Ordering::Relaxed) + 1;
        warn!(total_failed = n, "Event failed recorded");
    }

    /// Retrieve a snapshot of current event statistics.
    ///
    /// Returns an [`EventsStatsData`] struct containing the current values of all counters.
    /// Uses acquire ordering to ensure visibility of all prior increments across threads.
    ///
    /// # Returns
    ///
    /// An [`EventsStatsData`] snapshot containing current totals.
    pub fn get(&self) -> EventsStatsData {
        EventsStatsData {
            total_sent: self.total_sent.load(Ordering::Acquire),
            total_received: self.total_received.load(Ordering::Acquire),
            total_failed: self.total_failed.load(Ordering::Acquire),
        }
    }
}

/// Unified metrics collector for all transport message types.
///
/// This generic struct aggregates [`MessagesStats`], [`RequestsStats`], and [`EventsStats`]
/// into a single metrics container parameterized by a [`TransportSpec`] type. It provides
/// a cohesive interface for recording lifecycle events across all message kinds (requests,
/// responses, and events) and retrieving unified snapshots.
///
/// # Generic Parameters
///
/// * `S` - The transport specification type defining request, response, and event types.
///
/// # Examples
///
/// ```rust,ignore
/// let metrics = Arc::new(TransportMetrics::<MySpec>::default());
/// metrics.send(&msg).await;
/// let (msg_data, req_data, ev_data) = metrics.get().await;
/// ```
#[derive(Debug, Default)]
pub struct TransportMetrics<S: TransportSpec> {
    messages: MessagesStats,
    requests: RequestsStats,
    events: EventsStats,
    _marker: PhantomData<S>,
}
impl<S: TransportSpec> TransportMetrics<S> {
    /// Record that a message was sent.
    ///
    /// Increments the general message counter and dispatches to the appropriate
    /// message-kind-specific handler (request, event, etc.).
    ///
    /// # Arguments
    ///
    /// * `msg` - The message being sent.
    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "sent_metrics", level = "trace")]
    async fn send(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.send();
        match msg.kind {
            MessageKind::Request(_) => self.requests.send(msg.id()).await,
            MessageKind::Event(_) => self.events.send(),
            _ => trace!("Outbound message (non-tracked kind)"),
        }
    }

    /// Record that a message was received.
    ///
    /// Increments the general message counter and dispatches to the appropriate
    /// message-kind-specific handler (request, response, event, etc.).
    ///
    /// # Arguments
    ///
    /// * `msg` - The message being received.
    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "received_metrics", level = "trace")]
    async fn receive(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.receive();
        match msg.kind {
            MessageKind::Request(_) => self.requests.receive(msg.id()).await,
            MessageKind::Response(_) => self.requests.receive(msg.id()).await,
            MessageKind::Event(_) => self.events.receive(),
        }
    }

    /// Record that a message failed.
    ///
    /// Increments the general failure counter and dispatches to the appropriate
    /// message-kind-specific failure handler.
    ///
    /// # Arguments
    ///
    /// * `msg` - The message that failed.
    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "failed_metrics", level = "warn")]
    async fn fail(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.fail();
        match msg.kind {
            MessageKind::Request(_) => self.requests.fail(msg.id()).await,
            MessageKind::Response(_) => self.requests.fail(msg.id()).await,
            MessageKind::Event(_) => self.events.fail(),
        }
    }

    /// Retrieve a unified snapshot of all metrics.
    ///
    /// Returns a tuple of `(MessagesStatsData, RequestsStatsData, EventsStatsData)`
    /// representing the current state of all three metric categories.
    ///
    /// # Returns
    ///
    /// A tuple containing snapshots for messages, requests, and events.
    pub async fn get(&self) -> (MessagesStatsData, RequestsStatsData, EventsStatsData) {
        (
            self.messages.get(),
            self.requests.get().await,
            self.events.get(),
        )
    }
}

/// Middleware layer for transparent metrics collection.
///
/// This middleware component integrates [`TransportMetrics`] into the message processing
/// pipeline. It records metrics for inbound and outbound messages while propagating
/// the message to subsequent middleware or handlers.
///
/// # Examples
///
/// ```rust,ignore
/// let metrics = Arc::new(TransportMetrics::default());
/// let mw = MetricsMiddleware::new(metrics.clone());
/// // Add to middleware chain...
/// ```
#[derive(Debug, Default)]
pub struct MetricsMiddleware<S: TransportSpec>(Arc<TransportMetrics<S>>);

impl<S: TransportSpec> MetricsMiddleware<S> {
    /// Create a new metrics middleware from an existing metrics collector.
    ///
    /// # Arguments
    ///
    /// * `metrics` - An `Arc<TransportMetrics<S>>` to use for recording.
    ///
    /// # Returns
    ///
    /// A new `MetricsMiddleware` instance wrapping the provided metrics.
    pub fn new(metrics: Arc<TransportMetrics<S>>) -> Self {
        MetricsMiddleware(metrics)
    }
}

/// Implementation of the middleware pipeline for metrics recording.
///
/// This trait implementation allows `MetricsMiddleware` to intercept both inbound
/// and outbound messages, recording metrics before delegating to the next middleware.
/// If the downstream pipeline encounters an error, the failure is also recorded.
#[async_trait]
impl<R, S> Middleware<R, S> for MetricsMiddleware<S>
where
    R: RawTransport,
    S: TransportSpec,
{
    /// Handle an inbound message in the middleware chain.
    ///
    /// Records the received message and invokes the next middleware. If an error occurs,
    /// the message failure is recorded and the error is propagated.
    ///
    /// # Arguments
    ///
    /// * `msg` - The received message.
    /// * `next` - The next middleware in the chain.
    ///
    /// # Returns
    ///
    /// `Ok(())` if processing succeeds, or a [`TransportError`] if any middleware fails.
    #[instrument(skip(self, msg, next), fields(msg_id = msg.id(), kind = ?msg.kind), name = "metrics_middleware_recv", level = "trace")]
    async fn on_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
        next: Inbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        self.0.receive(msg).await;
        if let Err(e) = next.run(msg).await {
            warn!(error = %e, "Inbound pipeline error");
            self.0.fail(msg).await;
            return Err(e);
        }
        Ok(())
    }

    /// Handle an outbound message in the middleware chain.
    ///
    /// Records the sent message and invokes the next middleware. If an error occurs,
    /// the message failure is recorded and the error is propagated.
    ///
    /// # Arguments
    ///
    /// * `msg` - The message being sent (mutable to allow downstream modifications).
    /// * `next` - The next middleware in the chain.
    ///
    /// # Returns
    ///
    /// `Ok(())` if processing succeeds, or a [`TransportError`] if any middleware fails.
    #[instrument(skip(self, msg, next), fields(msg_id = msg.id(), kind = ?msg.kind), name = "metrics_middleware_send", level = "trace")]
    async fn on_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
        next: Outbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        self.0.send(msg).await;
        if let Err(e) = next.run(msg).await {
            warn!(error = %e, "Outbound pipeline error");
            self.0.fail(msg).await;
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{codec::JsonCodec, messages::Message};
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;

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

    #[derive(Debug, Clone, Default)]
    pub struct TestSpec;
    impl TransportSpec for TestSpec {
        type Req = TestReq;
        type Res = TestRes;
        type Ev = TestEv;
        type C = JsonCodec;
    }

    fn make_request(id: u32) -> Message<TestReq, TestRes, TestEv> {
        Message::request(id, TestReq::Ping)
    }

    fn make_response(id: u32) -> Message<TestReq, TestRes, TestEv> {
        Message::response(id, TestRes::Pong)
    }

    fn make_event() -> Message<TestReq, TestRes, TestEv> {
        Message::event(0, TestEv::Event)
    }

    mod messages_stats {
        use super::*;

        #[test]
        fn initial_state_is_zero() {
            let s = MessagesStats::default();
            let d = s.get();
            assert_eq!(d.total_sent, 0);
            assert_eq!(d.total_received, 0);
            assert_eq!(d.total_failed, 0);
        }

        #[test]
        fn send_increments_only_sent() {
            let s = MessagesStats::default();
            s.send();
            s.send();
            let d = s.get();
            assert_eq!(d.total_sent, 2);
            assert_eq!(d.total_received, 0);
            assert_eq!(d.total_failed, 0);
        }

        #[test]
        fn receive_increments_only_received() {
            let s = MessagesStats::default();
            s.receive();
            let d = s.get();
            assert_eq!(d.total_sent, 0);
            assert_eq!(d.total_received, 1);
            assert_eq!(d.total_failed, 0);
        }

        #[test]
        fn fail_increments_only_failed() {
            let s = MessagesStats::default();
            s.fail();
            let d = s.get();
            assert_eq!(d.total_sent, 0);
            assert_eq!(d.total_received, 0);
            assert_eq!(d.total_failed, 1);
        }

        #[test]
        fn independent_counters_accumulate_separately() {
            let s = MessagesStats::default();
            for _ in 0..5 {
                s.send();
            }
            for _ in 0..3 {
                s.receive();
            }
            for _ in 0..2 {
                s.fail();
            }
            let d = s.get();
            assert_eq!(d.total_sent, 5);
            assert_eq!(d.total_received, 3);
            assert_eq!(d.total_failed, 2);
        }

        #[tokio::test]
        async fn concurrent_sends_are_consistent() {
            let s = Arc::new(MessagesStats::default());
            let mut handles = vec![];
            for _ in 0..100 {
                let s = Arc::clone(&s);
                handles.push(tokio::spawn(async move {
                    s.send();
                }));
            }
            for h in handles {
                h.await.unwrap();
            }
            assert_eq!(s.get().total_sent, 100);
        }
    }

    mod requests_stats {
        use super::*;

        #[tokio::test]
        async fn initial_state() {
            let s = RequestsStats::default();
            let d = s.get().await;
            assert_eq!(d.total_sent, 0);
            assert_eq!(d.total_received, 0);
            assert_eq!(d.total_failed, 0);
            assert_eq!(d.total_retried, 0);
            assert_eq!(d.avg_latency, 0);
            assert_eq!(d.max_latency, 0);
            assert_eq!(d.min_latency, u64::MAX);
        }

        #[tokio::test]
        async fn send_increments_total_sent() {
            let s = RequestsStats::default();
            s.send(1).await;
            s.send(2).await;
            assert_eq!(s.total_sent.load(Ordering::Relaxed), 2);
        }

        #[tokio::test]
        async fn receive_known_request_increments_received() {
            let s = RequestsStats::default();
            s.send(42).await;
            s.receive(42).await;
            let d = s.get().await;
            assert_eq!(d.total_received, 1);
            assert_eq!(d.total_failed, 0);
        }

        #[tokio::test]
        async fn receive_unknown_id_does_not_increment_received_in_shared() {
            let s = RequestsStats::default();
            s.receive(99).await;
            assert_eq!(s.total_received.load(Ordering::Relaxed), 1);
            let d = s.get().await;
            assert_eq!(d.avg_latency, 0);
            assert_eq!(d.min_latency, u64::MAX);
        }

        #[tokio::test]
        async fn latency_is_measured_after_delay() {
            let s = RequestsStats::default();
            s.send(1).await;
            sleep(Duration::from_millis(20)).await;
            s.receive(1).await;

            let d = s.get().await;
            assert!(d.min_latency >= 5, "min_latency={}", d.min_latency);
            assert!(d.max_latency < 200, "max_latency={}", d.max_latency);
            assert!(d.avg_latency >= 5, "avg_latency={}", d.avg_latency);
        }

        #[tokio::test]
        async fn min_max_latency_tracking() {
            let s = RequestsStats::default();

            s.send(1).await;
            sleep(Duration::from_millis(10)).await;
            s.receive(1).await;

            s.send(2).await;
            sleep(Duration::from_millis(50)).await;
            s.receive(2).await;

            let d = s.get().await;
            assert!(d.min_latency <= d.max_latency, "min должен быть ≤ max");
            assert!(
                d.max_latency > d.min_latency,
                "после двух разных задержек они должны различаться"
            );
        }

        #[tokio::test]
        async fn avg_latency_welford_single_sample() {
            let s = RequestsStats::default();
            s.send(1).await;
            sleep(Duration::from_millis(30)).await;
            s.receive(1).await;

            let d = s.get().await;
            assert_eq!(d.avg_latency, d.min_latency);
            assert_eq!(d.avg_latency, d.max_latency);
        }

        #[tokio::test]
        async fn avg_latency_converges_between_min_and_max() {
            let s = RequestsStats::default();
            for id in 0..5_u32 {
                s.send(id).await;
                sleep(Duration::from_millis(10 + id as u64 * 5)).await;
                s.receive(id).await;
            }
            let d = s.get().await;
            assert!(d.avg_latency >= d.min_latency);
            assert!(d.avg_latency <= d.max_latency);
        }

        #[tokio::test]
        async fn fail_known_request_increments_failed() {
            let s = RequestsStats::default();
            s.send(7).await;
            s.fail(7).await;
            let d = s.get().await;
            assert_eq!(d.total_failed, 1);
            assert_eq!(d.total_received, 0);
        }

        #[tokio::test]
        async fn fail_unknown_id_is_noop() {
            let s = RequestsStats::default();
            s.fail(404).await; // не было send
            assert_eq!(s.total_failed.load(Ordering::Relaxed), 0);
        }

        #[tokio::test]
        async fn failed_request_removed_from_pending() {
            let s = RequestsStats::default();
            s.send(1).await;
            s.fail(1).await;
            s.fail(1).await;
            assert_eq!(s.total_failed.load(Ordering::Relaxed), 1);
        }

        #[tokio::test]
        async fn retry_counted_after_fail_and_resend() {
            let s = RequestsStats::default();
            s.send(1).await;
            s.fail(1).await;
            s.send(1).await;
            let d = s.get().await;
            assert_eq!(d.total_retried, 1);
        }

        #[tokio::test]
        async fn no_retry_without_prior_fail() {
            let s = RequestsStats::default();
            s.send(1).await;
            s.receive(1).await;
            s.send(1).await;
            let d = s.get().await;
            assert_eq!(d.total_retried, 0);
        }

        #[tokio::test]
        async fn multiple_retries_same_id() {
            let s = RequestsStats::default();
            for _ in 0..3 {
                s.send(1).await;
                s.fail(1).await;
            }
            s.send(1).await;
            let d = s.get().await;
            assert_eq!(d.total_retried, 3);
        }

        #[tokio::test]
        async fn double_receive_same_id_counts_once_in_latency() {
            let s = RequestsStats::default();
            s.send(1).await;
            s.receive(1).await;
            s.receive(1).await;

            let d = s.get().await;
            // avg/min/max не "удваиваются"
            assert_eq!(d.avg_latency, d.min_latency);
        }

        #[tokio::test]
        async fn concurrent_sends_and_receives() {
            let s = Arc::new(RequestsStats::default());
            let n = 50_u32;

            // отправляем все
            let mut handles = vec![];
            for id in 0..n {
                let s = Arc::clone(&s);
                handles.push(tokio::spawn(async move {
                    s.send(id).await;
                }));
            }
            for h in handles {
                h.await.unwrap();
            }

            // получаем все
            let mut handles = vec![];
            for id in 0..n {
                let s = Arc::clone(&s);
                handles.push(tokio::spawn(async move {
                    s.receive(id).await;
                }));
            }
            for h in handles {
                h.await.unwrap();
            }

            let d = s.get().await;
            assert_eq!(d.total_sent, n as u64);
            assert_eq!(d.total_received, n as u64);
            assert_eq!(d.total_failed, 0);
            assert!(d.avg_latency >= d.min_latency);
            assert!(d.avg_latency <= d.max_latency);
        }

        #[tokio::test]
        async fn concurrent_fails_do_not_double_count() {
            let s = Arc::new(RequestsStats::default());
            s.send(1).await;

            let s1 = Arc::clone(&s);
            let s2 = Arc::clone(&s);
            let (r1, r2) = tokio::join!(
                tokio::spawn(async move {
                    s1.fail(1).await;
                }),
                tokio::spawn(async move {
                    s2.fail(1).await;
                }),
            );
            r1.unwrap();
            r2.unwrap();

            assert_eq!(s.total_failed.load(Ordering::Relaxed), 1);
        }
    }

    mod events_stats {
        use super::*;

        #[test]
        fn initial_state_is_zero() {
            let s = EventsStats::default();
            let d = s.get();
            assert_eq!(d.total_sent, 0);
            assert_eq!(d.total_received, 0);
            assert_eq!(d.total_failed, 0);
        }

        #[test]
        fn send_receive_fail_are_independent() {
            let s = EventsStats::default();
            s.send();
            s.send();
            s.send();
            s.receive();
            s.receive();
            s.fail();
            let d = s.get();
            assert_eq!(d.total_sent, 3);
            assert_eq!(d.total_received, 2);
            assert_eq!(d.total_failed, 1);
        }

        #[tokio::test]
        async fn concurrent_increments() {
            let s = Arc::new(EventsStats::default());
            let mut handles = vec![];
            for _ in 0..200 {
                let s = Arc::clone(&s);
                handles.push(tokio::spawn(async move {
                    s.send();
                    s.receive();
                }));
            }
            for h in handles {
                h.await.unwrap();
            }
            let d = s.get();
            assert_eq!(d.total_sent, 200);
            assert_eq!(d.total_received, 200);
        }
    }

    mod transport_metrics {
        use super::*;

        #[tokio::test]
        async fn request_send_receive_happy_path() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_request(1)).await;
            m.receive(&make_response(1)).await;

            let (msg, req, _ev) = m.get().await;
            assert_eq!(msg.total_sent, 1);
            assert_eq!(msg.total_received, 1);
            assert_eq!(req.total_sent, 1);
            assert_eq!(req.total_received, 1);
            assert_eq!(req.total_failed, 0);
        }

        #[tokio::test]
        async fn request_send_fail() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_request(1)).await;
            m.fail(&make_request(1)).await;

            let (msg, req, _ev) = m.get().await;
            assert_eq!(msg.total_sent, 1);
            assert_eq!(msg.total_failed, 1);
            assert_eq!(req.total_failed, 1);
            assert_eq!(req.total_received, 0);
        }

        #[tokio::test]
        async fn event_send_receive() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_event()).await;
            m.receive(&make_event()).await;

            let (msg, _req, ev) = m.get().await;
            assert_eq!(msg.total_sent, 1);
            assert_eq!(msg.total_received, 1);
            assert_eq!(ev.total_sent, 1);
            assert_eq!(ev.total_received, 1);
        }

        #[tokio::test]
        async fn event_fail() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_event()).await;
            m.fail(&make_event()).await;

            let (_msg, _req, ev) = m.get().await;
            assert_eq!(ev.total_failed, 1);
        }

        #[tokio::test]
        async fn messages_counts_all_kinds() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_request(1)).await;
            m.send(&make_event()).await;
            m.receive(&make_response(1)).await;
            m.receive(&make_event()).await;
            m.fail(&make_request(4)).await;

            let (msg, _req, _ev) = m.get().await;
            assert_eq!(msg.total_sent, 2);
            assert_eq!(msg.total_received, 2);
            assert_eq!(msg.total_failed, 1);
        }

        #[tokio::test]
        async fn retry_visible_through_metrics() {
            let m = TransportMetrics::<TestSpec>::default();
            m.send(&make_request(1)).await;
            m.fail(&make_request(1)).await;
            m.send(&make_request(1)).await;
            m.receive(&make_response(1)).await;

            let (_msg, req, _ev) = m.get().await;
            assert_eq!(req.total_retried, 1);
            assert_eq!(req.total_received, 1);
        }

        #[tokio::test]
        async fn shared_arc_metrics_consistent() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());

            let m1 = Arc::clone(&metrics);
            let m2 = Arc::clone(&metrics);

            tokio::join!(
                async move {
                    m1.send(&make_request(1)).await;
                },
                async move {
                    m2.send(&make_event()).await;
                },
            );

            let (msg, req, ev) = metrics.get().await;
            assert_eq!(msg.total_sent, 2);
            assert_eq!(req.total_sent, 1);
            assert_eq!(ev.total_sent, 1);
        }
    }

    mod metrics_middleware {
        use super::*;
        use crate::{middleware::traits::Middleware, transport::RawTransport};
        use async_trait::async_trait;
        use std::sync::Arc;
        use tokio::sync::Mutex;
        use tokio::sync::mpsc;

        type TestMessage = Message<TestReq, TestRes, TestEv>;

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

        #[derive(Debug)]
        struct ErrorMw;

        #[async_trait]
        impl crate::middleware::traits::Middleware<MockRaw, TestSpec> for ErrorMw {
            async fn on_recv(
                &self,
                _msg: &TestMessage,
                _next: Inbound<'_, MockRaw, TestSpec>,
            ) -> Result<(), TransportError> {
                Err(TransportError::ConnectionClosed)
            }

            async fn on_send(
                &self,
                _msg: &mut TestMessage,
                _next: Outbound<'_, MockRaw, TestSpec>,
            ) -> Result<(), TransportError> {
                Err(TransportError::ConnectionClosed)
            }
        }

        #[derive(Debug)]
        struct SucceedingMw;
        impl Middleware<MockRaw, TestSpec> for SucceedingMw {}

        use crate::middleware::{NextState, Pipeline};

        fn state<'a>(
            pipeline: &'a Pipeline<MockRaw, TestSpec>,
        ) -> NextState<'a, MockRaw, TestSpec> {
            NextState {
                pipeline,
                next_index: 0,
            }
        }

        #[tokio::test]
        async fn on_send_success_increments_sent() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());
            let mw = MetricsMiddleware::new(Arc::clone(&metrics));

            let mut msg = make_request(1);
            let mut pipeline = Pipeline::default();
            pipeline.add_middleware(SucceedingMw);
            let mut state = state(&pipeline);

            let result = mw.on_send(&mut msg, Outbound(&mut state)).await;

            assert!(result.is_ok());
            let (m, _r, _e) = metrics.get().await;
            assert_eq!(m.total_sent, 1);
            assert_eq!(m.total_failed, 0);
        }

        #[tokio::test]
        async fn on_send_failure_increments_sent_and_failed() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());
            let mw = MetricsMiddleware::new(Arc::clone(&metrics));

            let mut msg = make_request(1);
            let mut pipeline = Pipeline::default();
            pipeline.add_middleware(ErrorMw);
            let mut state = state(&pipeline);
            let result = mw.on_send(&mut msg, Outbound(&mut state)).await;

            assert!(result.is_err());
            let (m, _r, _e) = metrics.get().await;
            assert_eq!(m.total_sent, 1);
            assert_eq!(m.total_failed, 1);
        }

        #[tokio::test]
        async fn on_recv_success_increments_received() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());
            let mw = MetricsMiddleware::new(Arc::clone(&metrics));

            let msg = make_response(1);
            let mut pipeline = Pipeline::default();
            pipeline.add_middleware(SucceedingMw);
            let mut state = state(&pipeline);
            let result = mw.on_recv(&msg, Inbound(&mut state)).await;

            assert!(result.is_ok());
            let (m, _r, _e) = metrics.get().await;
            assert_eq!(m.total_received, 1);
            assert_eq!(m.total_failed, 0);
        }

        #[tokio::test]
        async fn on_recv_failure_increments_received_and_failed() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());
            let mw = MetricsMiddleware::new(Arc::clone(&metrics));

            let msg = make_response(1);
            let mut pipeline = Pipeline::default();
            pipeline.add_middleware(ErrorMw);
            let mut state = state(&pipeline);
            let result = mw.on_recv(&msg, Inbound(&mut state)).await;

            assert!(result.is_err());
            let (m, _r, _e) = metrics.get().await;
            assert_eq!(m.total_received, 1);
            assert_eq!(m.total_failed, 1);
        }

        #[tokio::test]
        async fn latency_measured_via_middleware_roundtrip() {
            let metrics = Arc::new(TransportMetrics::<TestSpec>::default());
            let mw = MetricsMiddleware::new(Arc::clone(&metrics));

            // send
            let mut req = make_request(42);
            let mut pipeline = Pipeline::default();
            pipeline.add_middleware(SucceedingMw);
            let mut state1 = state(&pipeline);
            mw.on_send(&mut req, Outbound(&mut state1)).await.unwrap();

            sleep(Duration::from_millis(15)).await;

            let res = make_response(42);
            let mut pipeline2 = Pipeline::default();
            pipeline2.add_middleware(SucceedingMw);
            let mut state2 = state(&pipeline2);
            mw.on_recv(&res, Inbound(&mut state2)).await.unwrap();

            let (_m, r, _e) = metrics.get().await;
            assert!(r.avg_latency >= 5, "avg_latency={}", r.avg_latency);
        }
    }
}
