use crate::codec::MessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;

pub struct JsonCodec;

impl MessageCodec for JsonCodec {
    type Raw = Vec<u8>;

    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError> {
        serde_json::to_vec(message).map_err(|e| {
            TransportError::SerializeError { reason: e.to_string() }
        })
    }

    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError> {
        serde_json::from_slice(&raw).map_err(|e| {
            TransportError::DeserializeError { reason: e.to_string() }
        })
    }
}