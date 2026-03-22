use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;

mod json;
pub use json::JsonCodec;

pub trait MessageCodec {
    type Raw;

    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError>;
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError>;
}