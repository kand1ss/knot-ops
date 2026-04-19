use criterion::Criterion;
use knot_transport::codec::BinaryCodec;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{BenchEnv, SimpleReq, SimpleSpec, spawn_simple_echo_server};

pub fn bench_metadata_operations(c: &mut Criterion) {
    let env = BenchEnv::new();
    let path = env.unique_socket();
    let mut group = c.benchmark_group("metadata_ops");

    let (server, client) = env.runtime.block_on(async {
        let server_handle = spawn_simple_echo_server::<BinaryCodec>(&path).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<SimpleSpec<BinaryCodec>>();

        (server_handle, Arc::new(transport))
    });

    group.sample_size(200);
    group.bench_function("set_meta_small_key", |b| {
        let client = Arc::clone(&client);
        b.to_async(&env.runtime).iter(|| async {
            let client = Arc::clone(&client);
            let mut meta = knot_transport::messages::MetadataMap::new();
            meta.insert_str("trace_id", "123456789");

            let _ = black_box(client.request(SimpleReq::Ping, 30, Some(meta)).await);
        });
    });

    group.bench_function("set_meta_large_value", |b| {
        let client = Arc::clone(&client);
        b.to_async(&env.runtime).iter(|| async {
            let client = Arc::clone(&client);
            use knot_transport::messages::{MAX_METADATA_VALUE_LEN, MetadataMap};

            let mut meta = MetadataMap::new();
            meta.insert_str("context", "x".repeat(MAX_METADATA_VALUE_LEN));

            let _ = black_box(client.request(SimpleReq::Ping, 30, Some(meta)).await);
        });
    });

    group.finish();
    server.abort();
    env.cleanup_socket(&path);
}
