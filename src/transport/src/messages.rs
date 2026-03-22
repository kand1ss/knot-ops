//! # Knot Messages
//! 
//! This module defines the core message structures used for communication 
//! between the Knot CLI and the Knot Daemon.
//!
//! It implements a robust Request/Response pattern with correlation IDs 
//! and automatic timestamping.

use serde::{Serialize, Deserialize};
use knot_core::utils::TimestampUtils;

pub mod daemon;

/// The primary envelope for all communication.
///
/// `Message` wraps payloads of type `TRequest` and `TResponse` to provide
/// metadata required for tracking and synchronization.
#[derive(Debug, Serialize, Deserialize)]
pub struct Message<TRequest, TResponse> {
    /// Unique correlation ID to match responses to requests.
    pub id: u32,
    /// Unix timestamp in milliseconds, set at the moment of creation.
    pub timestamp: u64,
    /// The actual payload, either a Request or a Response.
    pub kind: MessageKind<TRequest, TResponse>,
}

/// Differentiates between outbound requests and inbound responses.
#[derive(Debug, Serialize, Deserialize)]
pub enum MessageKind<TRequest, TResponse> {
    /// A request sent from a client to a server.
    Request(TRequest),
    /// A response sent from a server back to a client.
    Response(TResponse),
}

impl<TRequest, TResponse> Message<TRequest, TResponse>
where
    TRequest: Serialize,
    TResponse: Serialize,
{
    /// Creates a new `Message` initialized as a **Request**.
    ///
    /// Automatically captures the current system time using `TimestampUtils`.
    ///
    /// # Arguments
    /// * `id` - A unique identifier for this request.
    /// * `payload` - The specific data for the request.
    pub fn request(id: u32, payload: TRequest) -> Self {
        Self {
            id,
            timestamp: TimestampUtils::now_ms(),
            kind: MessageKind::Request(payload),
        }
    }

    /// Creates a new `Message` initialized as a **Response**.
    ///
    /// The `id` should match the `id` of the request this response is addressing.
    ///
    /// # Arguments
    /// * `id` - The correlation ID from the original request.
    /// * `payload` - The specific data for the response.
    pub fn response(id: u32, payload: TResponse) -> Self {
        Self {
            id,
            timestamp: TimestampUtils::now_ms(),
            kind: MessageKind::Response(payload),
        }
    }

    /// Returns the correlation ID of this message.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Consumes the message and returns the ID and request payload.
    ///
    /// Returns `None` if the message is actually a `Response`.
    pub fn into_request(self) -> Option<(u32, TRequest)> {
        match self.kind {
            MessageKind::Request(payload) => Some((self.id, payload)),
            _ => None,
        }
    }

    /// Consumes the message and returns the ID and response payload.
    ///
    /// Returns `None` if the message is actually a `Request`.
    pub fn into_response(self) -> Option<(u32, TResponse)> {
        match self.kind {
            MessageKind::Response(payload) => Some((self.id, payload)),
            _ => None,
        }
    }

    /// Returns `true` if the message is a `Request`.
    pub fn is_request(&self) -> bool {
        matches!(self.kind, MessageKind::Request(_))
    }

    /// Returns `true` if the message is a `Response`.
    pub fn is_response(&self) -> bool {
        matches!(self.kind, MessageKind::Response(_))
    }
}