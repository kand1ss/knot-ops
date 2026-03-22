use crate::codec::MessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use bincode::{config, serde as bincode_serde};
use knot_core::errors::TransportError;


const BINCODE_CONFIG: config::Configuration<
    config::LittleEndian, 
    config::Fixint,
    config::Limit<4096>
> = config::standard()
    .with_little_endian()
    .with_fixed_int_encoding()
    .with_limit::<4096>();

    
pub struct BinaryCodec;
impl MessageCodec for BinaryCodec {
    type Raw = Vec<u8>;

    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError> {
        bincode_serde::encode_to_vec(message, BINCODE_CONFIG).map_err(|e| {
            TransportError::SerializeError { reason: e.to_string() }
        })
    }

    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError> {
        let (decoded, _): (T, usize) = bincode_serde::decode_from_slice(&raw, BINCODE_CONFIG).map_err(|e| {
            TransportError::DeserializeError { reason: e.to_string() }
        })?;
        
        Ok(decoded)
    }
}