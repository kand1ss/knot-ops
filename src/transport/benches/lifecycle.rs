use criterion::Criterion;
use knot_transport::codec::BinaryCodec;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::hint::black_box;
use std::time::Duration;

use crate::common::{BenchEnv, SimpleReq, SimpleSpec, spawn_simple_echo_server};

pub fn bench_connection_lifecycle(c: &mut Criterion) {
    let env = BenchEnv::new();
    let path = env.unique_socket();
    let mut group = c.benchmark_group("lifecycle");

    group.sample_size(50);
    group.measurement_time(Duration::from_secs(10));

    let server_handle = env.runtime.block_on(
        #[allow(clippy::async_yields_async)]
        async {
            let handle = spawn_simple_echo_server::<BinaryCodec>(&path).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            handle
        },
    );

    group.bench_function("connect_and_drop", |b| {
        let runtime = &env.runtime;
        let path = path.clone();

        b.to_async(runtime).iter_custom(|iters| {
            let path = path.clone();
            async move {
                let mut total_time = Duration::ZERO;

                for _ in 0..iters {
                    let start = std::time::Instant::now();

                    let transport_result = IpcTransport::connect(path.clone()).await;

                    if let Ok(transport) = transport_result {
                        let client = transport.to_messaged::<SimpleSpec<BinaryCodec>>();
                        let _ = black_box(client.request(SimpleReq::Ping, 30, None).await);
                    }

                    total_time += start.elapsed();

                    // Small yield to let OS clean up the named pipe resources
                    // This prevents hitting connection limits / ERROR_ACCESS_DENIED
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }

                total_time
            }
        });
    });

    group.finish();
    server_handle.abort();
    env.cleanup_socket(&path);
}
