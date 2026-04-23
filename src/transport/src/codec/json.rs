//! # JSON Serialization Codec
//!
//! This module provides a text-based implementation of the `MessageCodec`
//! trait using the `serde_json` crate.
//!
//! While less compact than binary formats, this codec is ideal for debugging,
//! logging, and ensuring compatibility with systems that prefer human-readable data.

use crate::codec::MessageCodec;
use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A codec that uses JSON for message serialization.
///
/// Messages encoded with this codec are standard UTF-8 JSON byte vectors.
/// This is particularly useful during development to inspect IPC traffic
/// using standard tools.
#[derive(Debug)]
pub struct JsonCodec;

impl MessageCodec for JsonCodec {
    /// The codec produces and consumes byte vectors containing JSON text.
    type Raw = Vec<u8>;

    /// Encodes a typed message into a JSON byte vector.
    ///
    /// # Errors
    /// Returns `TransportError::SerializeError` if the value contains
    /// types that cannot be represented in JSON (e.g., non-string keys in Maps).
    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError> {
        serde_json::to_vec(message).map_err(|e| TransportError::SerializeError {
            reason: e.to_string(),
        })
    }

    /// Decodes a JSON byte slice into a Rust data structure.
    ///
    /// # Errors
    /// Returns `TransportError::DeserializeError` if the JSON is malformed,
    /// contains invalid UTF-8, or does not match the expected structure of `T`.
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError> {
        serde_json::from_slice(&raw).map_err(|e| TransportError::DeserializeError {
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{
        MessageKind,
        daemon::{DaemonMessage, DaemonRequest, DaemonResponse},
    };

    #[test]
    fn test_json_codec_serialize_deserialize() {
        let msg: DaemonMessage = DaemonMessage::request(42, DaemonRequest::Down);

        let raw = JsonCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = JsonCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(decoded.kind, MessageKind::Request(_)));
    }

    #[test]
    fn test_json_deserialize_response() {
        let msg: DaemonMessage =
            DaemonMessage::response(42, DaemonResponse::Status { services: vec![] });

        let raw = JsonCodec::encode(&msg).unwrap();
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
