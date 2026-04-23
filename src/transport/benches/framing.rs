use criterion::Criterion;
use knot_transport::codec::BinaryCodec;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{BenchEnv, SimpleReq, SimpleSpec, spawn_simple_echo_server};

pub fn bench_raw_vs_messaged(c: &mut Criterion) {
    let env = Arc::new(BenchEnv::new());
    let path = env.unique_socket();
    let mut group = c.benchmark_group("framing_overhead");

    group.sample_size(100);
    group.measurement_time(Duration::from_secs(5));

    let (server, client) = env.runtime.block_on(async {
        let server_handle = spawn_simple_echo_server::<BinaryCodec>(&path).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client")
            .to_messaged::<SimpleSpec<BinaryCodec>>();

        (server_handle, Arc::new(transport))
    });

    group.bench_function("messaged_transport", |b| {
        b.to_async(&env.runtime).iter(|| {
            let client = Arc::clone(&client);
            async move {
                let _ = black_box(client.request(SimpleReq::Ping, 30, None).await);
            }
        });
    });

    server.abort();
    env.cleanup_socket(&path);

    let path = env.unique_socket();
    let pre_encoded_req: Vec<u8> = <BinaryCodec as knot_transport::codec::MessageCodec>::encode(
        &crate::common::create_simple_message(),
    )
    .unwrap();

    let pre_encoded_res: Vec<u8> = <BinaryCodec as knot_transport::codec::MessageCodec>::encode(
        &knot_transport::messages::Message::<
            crate::common::SimpleReq,
            crate::common::SimpleRes,
            crate::common::SimpleEv,
        >::response(1, crate::common::SimpleRes::Pong),
    )
    .unwrap();

    let server_path = path.clone();
    let (raw_server, raw_client) = env.runtime.block_on(async {
        let res_payload = pre_encoded_res.clone();
        let server_handle = tokio::spawn(async move {
            use knot_transport::transport::{RawTransport, Server};
            let server = knot_transport::transport::ipc::IpcServer::bind(server_path)
                .await
                .unwrap();

            while let Ok(transport) = server.accept().await {
                let res_payload = res_payload.clone();
                tokio::spawn(async move {
                    while transport.recv_frame().await.is_ok() {
                        let _ = transport.send_frame(&res_payload).await;
                    }
                });
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let transport = IpcTransport::connect(path.clone())
            .await
            .expect("Failed to connect client");
        (server_handle, Arc::new(transport))
    });

    group.bench_function("raw_transport", |b| {
        let req_payload = pre_encoded_req.clone();
        b.to_async(&env.runtime).iter(|| {
            let client = Arc::clone(&raw_client);
            let req = req_payload.clone();
            async move {
                use knot_transport::transport::RawTransport;
                let _ = client.send_frame(&req).await;
                let _ = black_box(client.recv_frame().await);
            }
        });
    });

    group.finish();
    raw_server.abort();
    env.cleanup_socket(&path);
}
