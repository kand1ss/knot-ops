use knot_transport::{
    codec::{BinaryCodec, JsonCodec, MessageCodec},
    messages::MessageKind,
    transport::{
        MessageTransport, Server, TransportSpec,
        ipc::{IpcServer, IpcTransport},
    },
};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Runtime;

// HELPER TRAITS & TYPES

/// Codec benchmark trait for generic codec comparison
pub trait BenchCodec: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static {
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

pub struct BenchEnv {
    pub runtime: Runtime,
    socket_counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl BenchEnv {
    pub fn new() -> Self {
        static GLOBAL_OFFSET: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let offset = GLOBAL_OFFSET.fetch_add(10000, std::sync::atomic::Ordering::SeqCst);

        Self {
            runtime: Runtime::new().unwrap(),
            socket_counter: Arc::new(std::sync::atomic::AtomicUsize::new(offset)),
        }
    }

    pub fn unique_socket(&self) -> PathBuf {
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

    pub fn cleanup_socket(&self, path: &PathBuf) {
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for BenchEnv {
    fn drop(&mut self) {
        let base = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("bench_socks");
        let _ = std::fs::remove_dir(base);
    }
}

// HELPER FUNCTIONS (minimal test doubles)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimpleReq {
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimpleRes {
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimpleEv {}

#[derive(Debug)]
pub struct SimpleSpec<C: MessageCodec<Raw = Vec<u8>>>(std::marker::PhantomData<C>);

impl<C: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static> TransportSpec for SimpleSpec<C> {
    type Req = SimpleReq;
    type Res = SimpleRes;
    type Ev = SimpleEv;
    type C = C;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LargePayloadReq {
    Payload(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LargePayloadRes {
    Echo(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LargePayloadEv {
    Payload(String),
}

#[derive(Debug)]
pub struct LargePayloadSpec;

impl TransportSpec for LargePayloadSpec {
    type Req = LargePayloadReq;
    type Res = LargePayloadRes;
    type Ev = LargePayloadEv;
    type C = BinaryCodec;
}

pub async fn spawn_simple_echo_server<C: BenchCodec>(
    path: &std::path::Path,
) -> tokio::task::JoinHandle<()> {
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let server = IpcServer::bind(path).await.unwrap();
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, SimpleSpec<C>>| {
                    while let Ok(mut ctx) = transport.next().await {
                        let _ = ctx.reply(SimpleRes::Pong).await;
                    }
                    Ok(())
                },
            )
            .await;
    })
}

pub async fn spawn_large_payload_server(path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let server = IpcServer::bind(path).await.unwrap();
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, LargePayloadSpec>| {
                    while let Ok(mut ctx) = transport.next().await {
                        if let MessageKind::Request(LargePayloadReq::Payload(s)) = ctx.kind() {
                            let _ = ctx.reply(LargePayloadRes::Echo(s.clone())).await;
                        }
                    }
                    Ok(())
                },
            )
            .await;
    })
}

pub fn create_simple_message() -> knot_transport::messages::Message<SimpleReq, SimpleRes, SimpleEv>
{
    knot_transport::messages::Message::request(1, SimpleReq::Ping)
}

pub fn create_large_payload_message(
    size: usize,
) -> knot_transport::messages::Message<LargePayloadReq, LargePayloadRes, LargePayloadEv> {
    knot_transport::messages::Message::request(1, LargePayloadReq::Payload("x".repeat(size)))
}
