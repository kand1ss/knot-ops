//! # knot-client
//!
//! A high-level client library for interacting with the `knot` daemon.
//!
//! This crate provides the `KnotClient` struct, which is used to communicate with
//! the daemon over IPC (Unix Domain Sockets or Named Pipes). It supports
//! automatic daemon launching, health checks, and lifecycle management.

mod builder;
mod client;
pub mod errors;
pub(crate) mod handles;
pub mod launcher;
pub mod policies;
pub mod states;

mod process;
#[cfg(test)]
pub mod test_utils;

pub use builder::ClientBuilder;
pub use client::KnotClient;
