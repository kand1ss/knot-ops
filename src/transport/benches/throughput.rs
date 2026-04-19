use criterion::{BenchmarkId, Criterion, Throughput};
use knot_transport::transport::ipc::{IpcServer, IpcTransport};
use knot_transport::transport::{RawTransport, Server};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{BenchEnv, LargePayloadReq, LargePayloadSpec, spawn_large_payload_server};

pub fn bench_payload_scaling(c: &mut Criterion) {
    let env = Arc::new(BenchEnv::new());
    let path = env.unique_socket();
    let mut group = c.benchmark_group("payload_scaling");

    group.sample_size(50);
    group.measurement_time(Duration::from_secs(5));

    let sizes = [
        ("1KB", 1024),
        ("64KB", 64 * 1024),
        ("256KB", 256 * 1024),
        ("1MB", 1024 * 1024),
        ("4MB", 4 * 1024 * 1024),
        ("8MB", 8 * 1024 * 1024),
    ];

    let (server, client) = env.runtime.block_on(async {
        let server_handle = spawn_large_payload_server(&path).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<LargePayloadSpec>();

        (server_handle, Arc::new(transport))
    });

    for (label, size) in sizes {
        let env = Arc::clone(&env);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("binary", label),
            &size,
            |b, &payload_size| {
                let runtime = &env.runtime;
                let client = Arc::clone(&client);

                b.to_async(runtime).iter_custom(|iters| {
                    let client = Arc::clone(&client);
                    let payload = "x".repeat(payload_size);

                    async move {
                        let start = std::time::Instant::now();

                        for _ in 0..iters {
                            let _ = black_box(
                                client
                                    .request(LargePayloadReq::Payload(payload.clone()), 60, None)
                                    .await,
                            );
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

pub fn bench_unidirectional_stream(c: &mut Criterion) {
    let env = Arc::new(BenchEnv::new());
    let path = env.unique_socket();
    let mut group = c.benchmark_group("unidirectional_emit");

    group.sample_size(50);
    group.measurement_time(Duration::from_secs(5));

    let sizes = [
        ("1KB", 1024),
        ("256KB", 256 * 1024),
        ("4MB", 4 * 1024 * 1024),
    ];

    let server_path = path.clone();
    let (server, client) = env.runtime.block_on(async {
        let server_handle = tokio::spawn(async move {
            let server = IpcServer::bind(server_path).await.unwrap();
            let _ = server
                .accept_with(
                    async |transport: knot_transport::transport::MessageTransport<
                        IpcTransport,
                        LargePayloadSpec,
                    >| {
                        while let Ok(_ctx) = transport.next().await {}
                        Ok(())
                    },
                )
                .await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<LargePayloadSpec>();

        (server_handle, Arc::new(transport))
    });

    for (label, size) in sizes {
        let env = Arc::clone(&env);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::new("emit_binary", label),
            &size,
            |b, &payload_size| {
                let runtime = &env.runtime;
                let client = Arc::clone(&client);

                b.to_async(runtime).iter_custom(|iters| {
                    let client = Arc::clone(&client);
                    let payload = "x".repeat(payload_size);

                    async move {
                        let start = std::time::Instant::now();

                        for _ in 0..iters {
                            let _ = black_box(
                                client
                                    .send(knot_transport::messages::Message::event(
                                        0,
                                        crate::common::LargePayloadEv::Payload(payload.clone()),
                                    ))
                                    .await,
                            );
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
