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

#[derive(Debug)]
pub struct MessagesStatsData {
    pub total_sent: u64,
    pub total_received: u64,
    pub total_failed: u64,
}

#[derive(Debug, Default)]
pub struct MessagesStats {
    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub total_failed: AtomicU64,
}
impl MessagesStats {
    #[instrument(skip(self), name = "message_sent_metrics", level = "trace")]
    fn send(&self) {
        let n = self.total_sent.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_sent = n, "Message sent recorded");
    }

    #[instrument(skip(self), name = "message_received_metrics", level = "trace")]
    fn receive(&self) {
        let n = self.total_received.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_received = n, "Message received recorded");
    }

    #[instrument(skip(self), name = "message_failed_metrics", level = "warn")]
    fn fail(&self) {
        let n = self.total_failed.fetch_add(1, Ordering::Relaxed) + 1;
        warn!(total_failed = n, "Message failed recorded");
    }

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

#[derive(Debug, Default)]
struct RequestsSharedState {
    pending_requests: HashMap<u32, Instant>,
    failed: HashSet<u32>,

    total_received: u64,
    avg_latency: u64,
    min_latency: u64,
    max_latency: u64,
}
impl RequestsSharedState {
    fn new() -> Self {
        Self {
            min_latency: u64::MAX,
            ..Default::default()
        }
    }
}

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

#[derive(Debug)]
pub struct RequestsStats {
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

#[derive(Debug)]
pub struct EventsStatsData {
    pub total_sent: u64,
    pub total_received: u64,
    pub total_failed: u64,
}

#[derive(Debug, Default)]
pub struct EventsStats {
    pub total_sent: AtomicU64,
    pub total_received: AtomicU64,
    pub total_failed: AtomicU64,
}
impl EventsStats {
    #[instrument(skip(self), name = "event_sent_metrics", level = "trace")]
    fn send(&self) {
        let n = self.total_sent.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_sent = n, "Event sent recorded");
    }

    #[instrument(skip(self), name = "event_received_metrics", level = "trace")]
    fn receive(&self) {
        let n = self.total_received.fetch_add(1, Ordering::Relaxed) + 1;
        trace!(total_received = n, "Event received recorded");
    }

    #[instrument(skip(self), name = "event_failed_metrics", level = "warn")]
    fn fail(&self) {
        let n = self.total_failed.fetch_add(1, Ordering::Relaxed) + 1;
        warn!(total_failed = n, "Event failed recorded");
    }

    pub fn get(&self) -> EventsStatsData {
        EventsStatsData {
            total_sent: self.total_sent.load(Ordering::Acquire),
            total_received: self.total_received.load(Ordering::Acquire),
            total_failed: self.total_failed.load(Ordering::Acquire),
        }
    }
}

#[derive(Debug, Default)]
pub struct TransportMetrics<S: TransportSpec> {
    messages: MessagesStats,
    requests: RequestsStats,
    events: EventsStats,
    _marker: PhantomData<S>,
}
impl<S: TransportSpec> TransportMetrics<S> {
    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "sent_metrics", level = "trace")]
    async fn send(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.send();
        match msg.kind {
            MessageKind::Request(_) => self.requests.send(msg.id()).await,
            MessageKind::Event(_) => self.events.send(),
            _ => trace!("Outbound message (non-tracked kind)"),
        }
    }

    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "received_metrics", level = "trace")]
    async fn receive(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.receive();
        match msg.kind {
            MessageKind::Request(_) => self.requests.receive(msg.id()).await,
            MessageKind::Response(_) => self.requests.receive(msg.id()).await,
            MessageKind::Event(_) => self.events.receive(),
        }
    }

    #[instrument(skip(self, msg), fields(msg_id = msg.id(), kind = ?msg.kind), name = "failed_metrics", level = "warn")]
    async fn fail(&self, msg: &Message<S::Req, S::Res, S::Ev>) {
        self.messages.fail();
        match msg.kind {
            MessageKind::Request(_) => self.requests.fail(msg.id()).await,
            MessageKind::Response(_) => self.requests.fail(msg.id()).await,
            MessageKind::Event(_) => self.events.fail(),
        }
    }

    pub async fn get(&self) -> (MessagesStatsData, RequestsStatsData, EventsStatsData) {
        (
            self.messages.get(),
            self.requests.get().await,
            self.events.get(),
        )
    }
}

#[derive(Debug, Default)]
pub struct MetricsMiddleware<S: TransportSpec>(Arc<TransportMetrics<S>>);

impl<S: TransportSpec> MetricsMiddleware<S> {
    pub fn new(metrics: Arc<TransportMetrics<S>>) -> Self {
        MetricsMiddleware(metrics)
    }
}

#[async_trait]
impl<R, S> Middleware<R, S> for MetricsMiddleware<S>
where
    R: RawTransport,
    S: TransportSpec,
{
    #[instrument(skip(self, msg, next), fields(msg_id = msg.id(), kind = ?msg.kind), name = "metrics_middleware_recv", level = "trace")]
    async fn on_recv(
        &self,
        msg: &Message<S::Req, S::Res, S::Ev>,
        next: Inbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        self.0.receive(msg).await;
        if let Err(e) = next.run(msg).await {
            warn!(error = %e, "inbound pipeline error");
            self.0.fail(msg).await;
            return Err(e);
        }
        Ok(())
    }

    #[instrument(skip(self, msg, next), fields(msg_id = msg.id(), kind = ?msg.kind), name = "metrics_middleware_send", level = "trace")]
    async fn on_send(
        &self,
        msg: &mut Message<S::Req, S::Res, S::Ev>,
        next: Outbound<'_, R, S>,
    ) -> Result<(), TransportError> {
        self.0.send(msg).await;
        if let Err(e) = next.run(msg).await {
            warn!(error = %e, "outbound pipeline error");
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
        Message::event(TestEv::Event)
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
