use criterion::{Criterion, criterion_group, criterion_main};

mod codec;
mod common;
mod concurrency;
mod framing;
mod lifecycle;
mod metadata;
mod middleware;
mod throughput;

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(100);
    targets =
        codec::bench_codecs_sequential,
        codec::bench_codec_operations,
        throughput::bench_payload_scaling,
        throughput::bench_unidirectional_stream,
        concurrency::bench_concurrent_throughput,
        metadata::bench_metadata_operations,
        lifecycle::bench_connection_lifecycle,
        middleware::bench_middleware_overhead,
        framing::bench_raw_vs_messaged
);

criterion_main!(benches);
