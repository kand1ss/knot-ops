use crate::codec::MessageCodec;
use knot_core::errors::TransportError;

pub struct BinaryCodec;
impl MessageCodec for JsonCodec {
    type Raw = Vec<u8>;

    fn encode(message: &T) -> Result<Self::Raw, TransportError> {
        bincode::serialize(message).map_err(|e| {
            TransportError::SerializeError { reason: e.to_string() }
        })
    }

    fn decode(raw: Self::Raw) -> Result<T, TransportError> {
        bincode::deserialize(&raw).map_err(|e| {
            TransportError::DeserializeError { reason: e.to_string() }
        })
    }
}