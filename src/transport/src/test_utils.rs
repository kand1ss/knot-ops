use crate::{
    codec::MessageCodec,
    messages::{Message, MessageContext, MessageKind},
    transport::{
        MessageTransport, Server, TransportSpec,
        ipc::{IpcServer, IpcTransport},
    },
};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::path::PathBuf;
use tokio::task::JoinHandle;

pub fn sock(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let thread_id = std::thread::current().id();
    path.push(format!("knot-test-{}-{:?}.sock", suffix, thread_id));
    path
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Req {
    Ping(i32),
}
#[derive(Serialize, Deserialize, Debug)]
pub enum Res {
    Pong(i32),
}
#[derive(Serialize, Deserialize, Debug)]
pub enum Ev {
    Event(i32),
}

#[derive(Debug)]
pub struct Spec<C: MessageCodec<Raw = Vec<u8>>>(PhantomData<C>);
impl<C: MessageCodec<Raw = Vec<u8>>> TransportSpec for Spec<C> {
    type Req = Req;
    type Res = Res;
    type Ev = Ev;
    type C = C;
}

pub type Trans<T, C> = MessageTransport<T, Spec<C>>;
pub type Msg = Message<Req, Res, Ev>;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum BigReq {
    Payload(String),
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum BigRes {
    Echo(String),
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum BigEv {}

#[derive(Debug)]
pub struct BigSpec<C: MessageCodec<Raw = Vec<u8>>>(PhantomData<C>);
impl<C: MessageCodec<Raw = Vec<u8>>> TransportSpec for BigSpec<C> {
    type Req = BigReq;
    type Res = BigRes;
    type Ev = BigEv;
    type C = C;
}

pub type BigTrans<C> = MessageTransport<IpcTransport, BigSpec<C>>;

pub async fn echo_server<Cod>(socket_path: PathBuf) -> JoinHandle<()>
where
    Cod: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let server = IpcServer::bind(socket_path).await.unwrap();
        server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, Spec<Cod>>| {
                    transport
                        .serve_with(
                            async |mut ctx: MessageContext<'_, IpcTransport, Spec<Cod>>| match ctx
                                .kind()
                            {
                                MessageKind::Request(req) => {
                                    let Req::Ping(val) = req;
                                    let mut current_val = *val;

                                    if let Some(metadata_val) = ctx.get_meta("increment")
                                        && let Ok(inc) = metadata_val.parse::<i32>()
                                    {
                                        current_val += inc;
                                        ctx.set_meta("incremented", "true").unwrap();
                                    }

                                    ctx.reply(Res::Pong(current_val)).await
                                }
                                MessageKind::Event(ev) => {
                                    let Ev::Event(val) = ev;
                                    let current_val = *val;

                                    if let Some(metadata_val) = ctx.get_meta("metadata")
                                        && let Ok(inc) = metadata_val.parse::<bool>()
                                        && inc
                                    {
                                        ctx.set_meta("metadata", "true").unwrap();
                                    }

                                    ctx.event(Ev::Event(current_val)).await
                                }
                                MessageKind::Response(_) => Ok(()),
                            },
                        )
                        .await
                },
            )
            .await
            .unwrap()
    })
}
