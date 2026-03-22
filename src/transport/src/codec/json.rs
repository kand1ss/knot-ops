use crate::codec::MessageCodec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use knot_core::errors::TransportError;

#[derive(Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{daemon::{DaemonMessage, DaemonRequest, DaemonResponse}, MessageKind};

    #[test]
    fn test_json_codec_serialize_deserialize() {
        let msg: DaemonMessage = DaemonMessage::request(
            42,
            DaemonRequest::Down,
        );

        let raw = JsonCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(decoded.kind, MessageKind::Request(_)));
    }

    #[test]
    fn test_json_deserialize_response() {
        let msg: DaemonMessage = DaemonMessage::response(
            42,
            DaemonResponse::Status { services: vec![] },
        );

        let raw     = JsonCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(
            decoded.kind,
            MessageKind::Response(DaemonResponse::Status { .. })
        ));
    }

    #[test]
    fn test_json_deserialize_invalid_data() {
        let invalid = b"not valid json at all {{{".to_vec();

        let result: Result<DaemonMessage, _> = JsonCodec::decode(invalid);

        assert!(matches!(
            result,
            Err(TransportError::DeserializeError { .. })
        ));
    }
}