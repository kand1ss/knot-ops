use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;

pub trait MessageCodec {
    type Raw;

    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError>;
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError>;
}