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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{daemon::{DaemonRequest, DaemonResponse, DaemonMessage}, MessageKind};

    #[test]
    fn test_binary_codec_serialize_deserialize() {
        let msg: DaemonMessage = DaemonMessage::request(
            42,
            DaemonRequest::Down,
        );

        let raw = BinaryCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = BinaryCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(decoded.kind, MessageKind::Request(_)));
    }

    #[test]
    fn test_binary_deserialize_response() {
        let msg: DaemonMessage = DaemonMessage::response(
            42,
            DaemonResponse::Status { services: vec![] },
        );

        let raw = BinaryCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = BinaryCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(
            decoded.kind,
            MessageKind::Response(DaemonResponse::Status { .. })
        ));
    }

    #[test]
    fn test_binary_deserialize_invalid_data() {
        let invalid = b"not valid data at all {{{".to_vec();
        let result: Result<DaemonMessage, _> = BinaryCodec::decode(invalid);

        assert!(matches!(
            result,
            Err(TransportError::DeserializeError { .. })
        ));
    }
}