//! # knot-client
//!
//! A high-level client library for interacting with the `knot` daemon.
//!
//! This crate provides the `KnotClient` struct, which is used to communicate with
//! the daemon over IPC (Unix Domain Sockets or Named Pipes). It supports
//! automatic daemon launching, health checks, and lifecycle management.

mod client;
pub mod launcher;
mod stream;
pub(crate) mod utils;

pub use client::KnotClient;
pub use stream::InboxStream;
