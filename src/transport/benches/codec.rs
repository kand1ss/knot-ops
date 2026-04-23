use criterion::{BenchmarkGroup, BenchmarkId, Criterion, measurement::WallTime};
use knot_transport::codec::{BinaryCodec, JsonCodec};
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{
    BenchCodec, BenchEnv, SimpleReq, SimpleSpec, create_large_payload_message,
    create_simple_message, spawn_simple_echo_server,
};

pub fn bench_codecs_sequential(c: &mut Criterion) {
    let env = BenchEnv::new();
    let mut group = c.benchmark_group("codec_sequential");

    group.sample_size(100);
    group.measurement_time(Duration::from_secs(10));
    group.warm_up_time(Duration::from_secs(2));

    test_codec_variant::<BinaryCodec>(&env, &mut group, "small");
    test_codec_variant::<JsonCodec>(&env, &mut group, "small");

    group.finish();
}

fn test_codec_variant<C: BenchCodec>(
    env: &BenchEnv,
    group: &mut BenchmarkGroup<'_, WallTime>,
    size_label: &str,
) {
    let path = env.unique_socket();
    let (server, client) = env.runtime.block_on(async {
        let server_handle = spawn_simple_echo_server::<C>(&path).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<SimpleSpec<C>>();

        (server_handle, Arc::new(transport))
    });

    group.bench_with_input(
        BenchmarkId::new(C::name(), size_label),
        &C::name(),
        |b, _| {
            let client = Arc::clone(&client);

            b.to_async(&env.runtime).iter(|| {
                let client = Arc::clone(&client);
                async move {
                    let _ = black_box(client.request(SimpleReq::Ping, 30, None).await);
                }
            });
        },
    );

    server.abort();
    env.cleanup_socket(&path);
}

pub fn bench_codec_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec_operations");

    group.sample_size(1000);

    group.bench_function("encode_small_binary", |b| {
        let msg = create_simple_message();
        b.iter(|| <BinaryCodec as knot_transport::codec::MessageCodec>::encode(black_box(&msg)));
    });

    group.bench_function("encode_small_json", |b| {
        let msg = create_simple_message();
        b.iter(|| <JsonCodec as knot_transport::codec::MessageCodec>::encode(black_box(&msg)));
    });

    group.bench_function("encode_large_binary", |b| {
        let msg = create_large_payload_message(1024 * 1024);
        b.iter(|| <BinaryCodec as knot_transport::codec::MessageCodec>::encode(black_box(&msg)));
    });

    group.bench_function("encode_large_json", |b| {
        let msg = create_large_payload_message(1024 * 1024);
        b.iter(|| <JsonCodec as knot_transport::codec::MessageCodec>::encode(black_box(&msg)));
    });

    group.finish();
}
