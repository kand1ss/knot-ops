//! # Knot Transport Library
//!
//! The `knot-transport` crate provides a high-level, asynchronous communication
//! infrastructure for the Knot process orchestrator.
//!
//! ## Overview
//!
//! This library is built on a layered architecture that separates data structures,
//! serialization logic, and I/O operations:
//!
//! 1.  **[`messages`]**: Defines the typed Request/Response envelopes used across the system.
//! 2.  **[`codec`]**: Provides traits and implementations for various serialization
//!     formats (e.g., Bincode for performance, JSON for compatibility).
//! 3.  **[`transport`]**: Implements the asynchronous engine that handles frame-based
//!     I/O, request-response correlation, and connection management.
//!
//! ## Key Patterns
//!
//! ### Type-Safe IPC
//! All communication is strictly typed. By combining `Message` structures with
//! a `MessageCodec`, the transport layer ensures that only valid, schema-compliant
//! data is processed.
//!
//! ### Asynchronous Multiplexing
//! The transport layer uses a background worker pattern. This allows a single
//! connection to handle multiple concurrent requests without blocking the
//! main execution thread.
//!
//! ## Example
//!
//! ```rust
//! // Converting a raw UnixStream into a high-level typed transport
//! let transport = raw_socket.to_messaged::<DaemonRequest, DaemonResponse, BinaryCodec>();
//!
//! // Sending a request and waiting for a specific response
//! let response = transport.request(DaemonRequest::Status).await?;
//! ```

/// Serialization and deserialization logic.
///
/// This module contains the `MessageCodec` trait and standard implementations
/// like `BinaryCodec` (Bincode v3) and `JsonCodec`.
pub mod codec;

/// Data structures for IPC communication.
///
/// Defines the `Message` and `MessageKind` types which act as the primary
/// communication protocol between the Knot CLI and Daemon.
pub mod messages;

/// Asynchronous I/O and transport abstractions.
///
/// Provides the `RawTransport` and `MessageTransport` types, as well as
/// the `Server` trait for building listening services (e.g., Unix Domain Sockets).
pub mod transport;
