use criterion::{BenchmarkId, Criterion, Throughput};
use knot_transport::codec::BinaryCodec;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{BenchEnv, SimpleReq, SimpleSpec, spawn_simple_echo_server};

pub fn bench_concurrent_throughput(c: &mut Criterion) {
    let env = Arc::new(BenchEnv::new());
    let path = env.unique_socket();
    let mut group = c.benchmark_group("concurrent_throughput");

    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    let (server, client) = env.runtime.block_on(async {
        let server_handle = spawn_simple_echo_server::<BinaryCodec>(&path).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<SimpleSpec<BinaryCodec>>();

        (server_handle, Arc::new(transport))
    });

    let concurrency_levels = [10, 50, 100, 500, 1000];

    for concurrency in concurrency_levels {
        group.throughput(Throughput::Elements(concurrency));

        group.bench_with_input(
            BenchmarkId::new("buffer_unordered", concurrency),
            &concurrency,
            |b, &buffer_size| {
                b.to_async(&env.runtime).iter_custom(|iters| {
                    let client = Arc::clone(&client);
                    async move {
                        use futures::stream::{self, StreamExt};

                        let start = std::time::Instant::now();
                        let batches = (iters / buffer_size).max(1);

                        for _ in 0..batches {
                            let _: Vec<_> = stream::iter(0..buffer_size)
                                .map(|_| {
                                    let c = Arc::clone(&client);
                                    async move { c.request(SimpleReq::Ping, 30, None).await }
                                })
                                .buffer_unordered(buffer_size as usize)
                                .collect()
                                .await;
                        }

                        start.elapsed()
                    }
                });
            },
        );
    }

    group.finish();
    server.abort();
    env.cleanup_socket(&path);
}
