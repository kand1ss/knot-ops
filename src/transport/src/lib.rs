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
//! 1.  **[`messages`]**: Defines the typed Request, Response, and Event envelopes.
//! 2.  **[`codec`]**: Provides traits and implementations for various serialization
//!     formats (e.g., Bincode for performance, JSON for compatibility).
//! 3.  **[`transport`]**: Implements the asynchronous engine that handles frame-based
//!     I/O, request-response correlation, and asynchronous event dispatching.
//!
//! ## Key Patterns
//!
//! ### Type-Safe IPC
//! All communication is strictly typed. By combining `Message` structures with
//! a `MessageCodec`, the transport layer ensures that only valid, schema-compliant
//! data is processed.
//!
//! ### Asynchronous Multiplexing
//! The transport layer is designed for non-blocking I/O. It allows a single
//! connection to process multiple concurrent operations, ensuring that slow
//! service startups or heavy log streams do not stall the CLI or Daemon.

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

/// Type aliases and convenience definitions for the Knot protocol.
///
/// This module provides specialized type aliases for `MessageTransport`
/// pre-configured with the standard Knot Daemon protocol types
/// (`DaemonRequest`, `DaemonResponse`, and `DaemonEvent`).
pub mod types;
