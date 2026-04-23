use criterion::{BenchmarkId, Criterion};
use knot_transport::codec::BinaryCodec;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{BenchEnv, SimpleReq, SimpleSpec};

pub fn bench_middleware_overhead(c: &mut Criterion) {
    let env = BenchEnv::new();
    let mut group = c.benchmark_group("middleware_overhead");

    let layer_counts = [0, 1, 5, 10];

    for count in layer_counts {
        group.bench_with_input(
            BenchmarkId::new("passthrough_layers", count),
            &count,
            |b, &layers| {
                let path = env.unique_socket();
                let (server, client) = env.runtime.block_on(async {
                    let server_handle =
                        crate::common::spawn_simple_echo_server::<BinaryCodec>(&path).await;
                    tokio::time::sleep(Duration::from_millis(50)).await;

                    let mut transport = IpcTransport::connect(path.clone())
                        .await
                        .expect("Failed to connect client")
                        .to_messaged::<SimpleSpec<BinaryCodec>>();

                    for _ in 0..layers {
                        transport.add_middleware(PassthroughMiddleware).await;
                    }

                    (server_handle, Arc::new(transport))
                });

                b.to_async(&env.runtime).iter(|| {
                    let client = Arc::clone(&client);
                    async move {
                        let _ = black_box(client.request(SimpleReq::Ping, 30, None).await);
                    }
                });

                server.abort();
                env.cleanup_socket(&path);
            },
        );
    }

    group.finish();
}

// Simple middleware that does nothing but pass the message along
struct PassthroughMiddleware;

impl std::fmt::Debug for PassthroughMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PassthroughMiddleware")
    }
}

#[async_trait::async_trait]
impl<R, S> knot_transport::middleware::traits::Middleware<R, S> for PassthroughMiddleware
where
    R: knot_transport::transport::RawTransport + Send + Sync + 'static,
    S: knot_transport::transport::TransportSpec + Send + Sync + 'static,
    S::Req: Send + Sync + 'static,
    S::Res: Send + Sync + 'static,
    S::Ev: Send + Sync + 'static,
{
    async fn on_send(
        &self,
        msg: &mut knot_transport::messages::Message<S::Req, S::Res, S::Ev>,
        next: knot_transport::middleware::Outbound<'_, R, S>,
    ) -> Result<(), knot_core::errors::TransportError> {
        next.run(msg).await
    }

    async fn on_recv(
        &self,
        msg: &knot_transport::messages::Message<S::Req, S::Res, S::Ev>,
        next: knot_transport::middleware::Inbound<'_, R, S>,
    ) -> Result<(), knot_core::errors::TransportError> {
        next.run(msg).await
    }
}
