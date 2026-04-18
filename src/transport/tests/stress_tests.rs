use knot_core::errors::TransportError;
use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::{MessageContext, MessageKind},
    test_utils::*,
    transport::{
        RawTransport, Server,
        ipc::{IpcServer, IpcTransport},
    },
};
use rstest::*;
use std::{marker::PhantomData, sync::Arc, time::Instant};
use tokio::{task::JoinHandle, time::Duration};

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test(flavor = "multi_thread", worker_threads = 16)]
async fn stress_concurrent_requests_10k<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("stress_10k");
    let server = echo_server::<Cod>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Arc<Trans<IpcTransport, Cod>> =
        Arc::new(IpcTransport::connect(path).await.unwrap().to_messaged());

    let start = Instant::now();
    let mut handles = vec![];

    // 10,000 concurrent requests
    for i in 0..10_000 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            match c.request(Req::Ping(i), 30, None).await {
                Ok(Res::Pong(v)) => assert_eq!(v, i),
                Err(e) => panic!("Request {i} failed: {e}"),
            }
        }));
    }

    for h in handles {
        h.await.expect("Task panicked");
    }

    let elapsed = start.elapsed();
    println!("10k concurrent requests: {elapsed:?}");
    println!("Throughput: {:.0} req/s", 10_000.0 / elapsed.as_secs_f64());

    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[test]
fn stress_codec_roundtrip<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let mb_sizes = [1, 2, 4, 8];
    for size in mb_sizes {
        let data = "x".repeat(size * 1024 * 1024);

        let t1 = Instant::now();
        let encoded = Cod::encode(&data).unwrap();
        let encode_time = t1.elapsed();
        let encoded_len = encoded.len();

        let t2 = Instant::now();
        let decoded: String = Cod::decode(encoded).unwrap();
        let decode_time = t2.elapsed();

        let decoded_len = Cod::encode(&decoded).unwrap().len();
        assert_eq!(encoded_len, decoded_len);
        println!("{size}MB: encode={encode_time:?}, decode={decode_time:?}");
    }
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test(flavor = "multi_thread")]
async fn stress_sequential_requests_10k<Cod>(#[case] _marker: PhantomData<Cod>)
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("stress_seq_100k");
    let server = echo_server::<Cod>(path.clone()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client: Trans<IpcTransport, Cod> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let start = Instant::now();

    // 100,000 sequential requests
    for i in 0..10_000 {
        match client.request(Req::Ping(i), 30, None).await {
            Ok(Res::Pong(v)) => {
                assert_eq!(v % 10_000, i % 10_000);
            }
            Err(e) => panic!("Request {i} failed: {e}"),
        }

        if i % 1_000 == 0 {
            println!("Progress: {i}/10000");
        }
    }

    let elapsed = start.elapsed();
    println!("100k sequential requests: {elapsed:?}");
    println!("Throughput: {:.0} req/s", 100_000.0 / elapsed.as_secs_f64());

    server.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test(flavor = "multi_thread")]
async fn stress_large_payloads<C>(#[case] _marker: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("large_payload");

    let srv: tokio::task::JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server: IpcServer = IpcServer::bind(path).await.unwrap();
            let t: BigTrans<C> = server.accept().await.unwrap().to_messaged();
            t.serve_with(
                async |mut ctx: MessageContext<'_, IpcTransport, BigSpec<C>>| {
                    let res: Result<(), TransportError> =
                        if let MessageKind::Request(BigReq::Payload(s)) = ctx.kind() {
                            ctx.reply(BigRes::Echo(s.clone())).await
                        } else {
                            Ok(())
                        };
                    res
                },
            )
            .await
            .ok();
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    let client: BigTrans<C> = IpcTransport::connect(path).await.unwrap().to_messaged();

    let start = Instant::now();

    // Test with increasing sizes: 1MB, 2MB, 4MB, 8MB
    for size_mb in [1, 2, 4, 8] {
        let payload = "x".repeat(size_mb * 1024 * 1024);

        match client
            .request(BigReq::Payload(payload.clone()), 60, None)
            .await
        {
            Ok(BigRes::Echo(echo)) => {
                assert_eq!(echo.len(), payload.len());
            }
            Err(e) => panic!("Failed for {size_mb}MB: {e}"),
        }

        println!("{size_mb}MB roundtrip: OK");
    }

    println!("All payload tests: {:?}", start.elapsed());
    srv.abort();
}

#[rstest]
#[case::json(PhantomData::<JsonCodec>)]
#[case::binary(PhantomData::<BinaryCodec>)]
#[tokio::test]
async fn stress_connection_churn<C>(#[case] _marker: PhantomData<C>)
where
    C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    let path = sock("stress_churn");
    let server: JoinHandle<()> = tokio::spawn({
        let path = path.clone();
        async move {
            let server = IpcServer::bind(path).await.unwrap();
            loop {
                if let Ok(raw) = server.accept().await {
                    let t: Trans<IpcTransport, C> = raw.to_messaged();
                    tokio::spawn(async move {
                        t.serve_with(async |mut ctx: MessageContext<'_, IpcTransport, Spec<C>>| {
                            if let MessageKind::Request(Req::Ping(i)) = ctx.kind() {
                                ctx.reply(Res::Pong(*i)).await.ok();
                            }
                            Ok(())
                        })
                        .await
                        .ok();
                    });
                }
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let start = Instant::now();

    // Create and destroy 1000 connections
    for i in 0..1000 {
        let raw = IpcTransport::connect(path.clone()).await.unwrap();
        let client = raw.to_messaged::<Spec<C>>();

        client.request(Req::Ping(0), 10, None).await.ok();
        drop(client);

        if i % 100 == 0 {
            println!("Created/destroyed {i} connections");
        }
    }

    println!("Connection churn (1000x): {:?}", start.elapsed());
    server.abort();
}
