//! # Binary Serialization Codec
//!
//! This module provides a high-performance binary implementation of the `MessageCodec`
//! trait using the `bincode` serialization format (version 3).
//!
//! It is optimized for Inter-Process Communication (IPC) within the Knot ecosystem.

use crate::{codec::MessageCodec, transport::MAX_MESSAGE_SIZE};
use bincode::{config, serde as bincode_serde};
use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Specific configuration for the Bincode engine.
///
/// * **Little Endian**: Standardizes byte order across different CPU architectures.
/// * **Fixed Integer Encoding**: Uses a fixed size for integers (faster but potentially
///   less compact than variable encoding for small numbers).
const BINCODE_CONFIG: config::Configuration<
    config::LittleEndian,
    config::Fixint,
    config::Limit<MAX_MESSAGE_SIZE>,
> = config::standard()
    .with_little_endian()
    .with_fixed_int_encoding()
    .with_limit::<MAX_MESSAGE_SIZE>();

/// A binary codec that uses `bincode` for serialization.
///
/// This codec is the default for production IPC in Knot due to its
/// minimal overhead and high throughput compared to text-based formats.
#[derive(Debug)]
pub struct BinaryCodec;

impl MessageCodec for BinaryCodec {
    /// The codec produces and consumes standard byte vectors.
    type Raw = Vec<u8>;

    /// Encodes a typed message into a binary `Vec<u8>`.
    ///
    /// # Errors
    /// Returns `TransportError::SerializeError` if the message exceeds 4KB
    /// or contains types incompatible with the bincode configuration.
    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError> {
        bincode_serde::encode_to_vec(message, BINCODE_CONFIG).map_err(|e| {
            TransportError::SerializeError {
                reason: e.to_string(),
            }
        })
    }

    /// Decodes a binary slice into a Rust data structure.
    ///
    /// Returns the decoded object of type `T`. Note that this implementation
    /// discards the number of bytes read provided by bincode, as the
    /// `IpcTransport` already handles frame boundaries.
    ///
    /// # Errors
    /// Returns `TransportError::DeserializeError` if the binary data is
    /// malformed or does not match the schema of `T`.
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError> {
        let (decoded, _): (T, usize) = bincode_serde::decode_from_slice(&raw, BINCODE_CONFIG)
            .map_err(|e| TransportError::DeserializeError {
                reason: e.to_string(),
            })?;

        Ok(decoded)
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
    fn test_binary_codec_serialize_deserialize() {
        let msg: DaemonMessage = DaemonMessage::request(42, DaemonRequest::Down);

        let raw = BinaryCodec::encode(&msg).unwrap();
        let decoded: DaemonMessage = BinaryCodec::decode(raw).unwrap();

        assert_eq!(decoded.id, 42);
        assert!(matches!(decoded.kind, MessageKind::Request(_)));
    }

    #[test]
    fn test_binary_deserialize_response() {
        let msg: DaemonMessage =
            DaemonMessage::response(42, DaemonResponse::Status { services: vec![] });

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
