//! # Knot Core Library
//!
//! `knot-core` is the foundational library for the Knot process orchestrator.
//!
//! It defines the shared domain models, states, error types, and
//! utility functions used by both the Daemon and the CLI. This crate is
//! designed to be strictly platform-independent where possible, focusing
//! on business logic and data integrity.
//!
//! ## Architecture Components
//!
//! The core is divided into several specialized modules:
//!
//! * **Data & States**: Defines what a "Service" is ([`data`][data]) and the
//!   lifecycle stages it can inhabit ([`states`][states]).
//! * **Configuration**: Handles the parsing and validation of service
//!   definitions ([`config`][config]).
//! * **Error Handling**: Provides a unified error system ([`errors`][errors])
//!   used across the entire workspace.
//! * **Observability**: Manages internal system events ([`events`][events])
//!   for logging and monitoring.
//!
//! ## Dependency Flow
//!
//! This crate is a leaf dependency for most other crates in the workspace
//! (like `knot-transport` or `knot-daemon`), ensuring a "single source of truth"
//! for the orchestrator's logic.

/// Configuration parsing and validation logic.
pub mod config;

/// Core data structures.
///
/// Defines the `ServiceData` struct, which tracks PIDs, names,
/// and other runtime-specific information for managed processes.
pub mod data;

/// Unified error handling.
///
/// Defines the `KnotError`, `TransportError`, and other specialized
/// error types using the `thiserror` pattern.
pub mod errors;

/// Internal system events.
///
/// Provides the event bus logic used to notify different parts of the
/// system about service starts, stops, or failures.
pub mod events;

/// Lifecycle states.
///
/// Defines the `ServiceStatus` enum (Starting, Running, Stopped, etc.)
/// and the rules for transitioning between them.
pub mod states;

/// Common helper functions.
///
/// Includes timestamp formatting, filesystem helpers, and other
/// low-level utilities used throughout the project.
pub mod utils;
