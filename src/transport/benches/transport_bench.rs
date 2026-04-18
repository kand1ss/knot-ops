use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::MessageKind,
    transport::{
        RawTransport, Server,
        ipc::{IpcServer, IpcTransport},
    },
};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

// HELPER TRAITS & TYPES

/// Codec benchmark trait for generic codec comparison
trait BenchCodec: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static {
    fn name() -> &'static str;
}

impl BenchCodec for BinaryCodec {
    fn name() -> &'static str {
        "binary"
    }
}

impl BenchCodec for JsonCodec {
    fn name() -> &'static str {
        "json"
    }
}

// SETUP & FIXTURES

struct BenchEnv {
    runtime: Runtime,
    socket_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl BenchEnv {
    fn new() -> Self {
        Self {
            runtime: Runtime::new().unwrap(),
            socket_counter: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn unique_socket(&self) -> PathBuf {
        let id = self
            .socket_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("bench_socks");
        let _ = std::fs::create_dir_all(&base);
        base.join(format!("knot_{}_{}.sock", std::process::id(), id))
    }

    fn cleanup_socket(&self, _path: &PathBuf) {
        let _ = std::fs::remove_file(_path);
    }
}

fn bench_codecs_sequential(c: &mut Criterion) {
    let env = BenchEnv::new();
    let mut group = c.benchmark_group("codec_sequential");

    group.sample_size(100); // More samples for stability
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

fn bench_payload_scaling(c: &mut Criterion) {
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

fn bench_concurrent_throughput(c: &mut Criterion) {
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

fn bench_metadata_operations(c: &mut Criterion) {
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

fn bench_codec_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec_operations");

    group.sample_size(1000);

    group.bench_function("encode_small_binary", |b| {
        let msg = create_simple_message();
        b.iter(|| BinaryCodec::encode(black_box(&msg)));
    });

    group.bench_function("encode_small_json", |b| {
        let msg = create_simple_message();
        b.iter(|| JsonCodec::encode(black_box(&msg)));
    });

    group.bench_function("encode_large_binary", |b| {
        let msg = create_large_payload_message(1024 * 1024);
        b.iter(|| BinaryCodec::encode(black_box(&msg)));
    });

    group.bench_function("encode_large_json", |b| {
        let msg = create_large_payload_message(1024 * 1024);
        b.iter(|| JsonCodec::encode(black_box(&msg)));
    });

    group.finish();
}

// HELPER FUNCTIONS (minimal test doubles)

use knot_transport::transport::{MessageTransport, TransportSpec};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SimpleReq {
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SimpleRes {
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SimpleEv {}

#[derive(Debug)]
struct SimpleSpec<C: MessageCodec<Raw = Vec<u8>>>(std::marker::PhantomData<C>);

impl<C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static> TransportSpec for SimpleSpec<C> {
    type Req = SimpleReq;
    type Res = SimpleRes;
    type Ev = SimpleEv;
    type C = C;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LargePayloadReq {
    Payload(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LargePayloadRes {
    Echo(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LargePayloadEv {}

#[derive(Debug)]
struct LargePayloadSpec;

impl TransportSpec for LargePayloadSpec {
    type Req = LargePayloadReq;
    type Res = LargePayloadRes;
    type Ev = LargePayloadEv;
    type C = BinaryCodec;
}

async fn spawn_simple_echo_server<C: BenchCodec>(
    path: &std::path::Path,
) -> tokio::task::JoinHandle<()> {
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let server = IpcServer::bind(path).await.unwrap();
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, SimpleSpec<C>>| {
                    while let Ok(mut ctx) = transport.next().await {
                        ctx.reply(SimpleRes::Pong).await?;
                    }
                    Ok(())
                },
            )
            .await;
    })
}

async fn spawn_large_payload_server(path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let server = IpcServer::bind(path).await.unwrap();
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, LargePayloadSpec>| {
                    while let Ok(mut ctx) = transport.next().await {
                        if let MessageKind::Request(LargePayloadReq::Payload(s)) = ctx.kind() {
                            ctx.reply(LargePayloadRes::Echo(s.clone())).await?;
                        }
                    }
                    Ok(())
                },
            )
            .await;
    })
}

fn create_simple_message() -> knot_transport::messages::Message<SimpleReq, SimpleRes, SimpleEv> {
    knot_transport::messages::Message::request(1, SimpleReq::Ping)
}

fn create_large_payload_message(
    size: usize,
) -> knot_transport::messages::Message<LargePayloadReq, LargePayloadRes, LargePayloadEv> {
    knot_transport::messages::Message::request(1, LargePayloadReq::Payload("x".repeat(size)))
}

// CRITERION MAIN

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(100);
    targets =
        bench_codecs_sequential,
        bench_payload_scaling,
        bench_concurrent_throughput,
        bench_metadata_operations,
        bench_codec_operations
);

criterion_main!(benches);
