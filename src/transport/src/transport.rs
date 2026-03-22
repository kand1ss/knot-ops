use crate::messages::Message;
use crate::codec::MessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;
use async_trait::async_trait;

pub mod ipc;

#[async_trait]
pub trait Transport<TRequest, TResponse, TCodec: MessageCodec> 
where
    TRequest: Serialize + DeserializeOwned + Send + 'static,
    TResponse: Serialize + DeserializeOwned + Send + 'static, 
{
    type Address: Send;

    // TODO - better abstraction (serialization by protocol by default)
    async fn connect(addr: Self::Address) -> Result<Self, TransportError> where Self: Sized;
    async fn serve(&self) -> Result<Message<TRequest, TResponse>, TransportError>;
    async fn send(&self, message: Message<TRequest, TResponse>) -> Result<(), TransportError>;
    async fn request(&self, request: TRequest) -> Result<TResponse, TransportError>;
}

#[async_trait]
pub trait Server {
    type Address: Send;

    async fn bind(addr: Self::Address) -> Result<Self, TransportError> 
    where 
        Self: Sized;

    async fn shutdown(&mut self);
    async fn accept<TRequest, TResponse, TCodec>(&self) 
        -> Result<Box<dyn Transport<TRequest, TResponse, TCodec, Address = Self::Address>>, TransportError>
    where
        TRequest: Serialize + DeserializeOwned + Send + Sync + 'static,
        TResponse: Serialize + DeserializeOwned + Send + Sync + 'static,
        TCodec: MessageCodec<Raw = Vec<u8>> + Send + Sync + 'static;
}