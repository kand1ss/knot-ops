//! # Message Serialization Codecs
//!
//! This module provides the `MessageCodec` trait and its implementations.
//! Codecs are responsible for converting typed messages into raw frames
//! and vice-versa.

use knot_core::errors::TransportError;
use serde::Serialize;
use serde::de::DeserializeOwned;

mod bincode;
mod json;

pub use bincode::BinaryCodec;
pub use json::JsonCodec;

/// A trait for types that can encode and decode messages.
///
/// Implementations of this trait define how a specific data format
/// (like JSON or Bincode) handles the transformation of Rust types
/// into a `Raw` format (typically `Vec<u8>`).
pub trait MessageCodec {
    /// The resulting type of the encoding process.
    /// Usually `Vec<u8>` for network or IPC transports.
    type Raw;

    /// Encodes a serializable value into the codec's raw format.
    ///
    /// # Errors
    /// Returns `TransportError::SerializeError` if the value cannot be serialized.
    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError>;

    /// Decodes a value from the codec's raw format into a Rust structure.
    ///
    /// # Errors
    /// Returns `TransportError::DeserializeError` if the raw data is invalid
    /// or does not match the expected type `T`.
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError>;
}
