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

/// Common helper functions.
///
/// Includes timestamp formatting, filesystem helpers, and other
/// low-level utilities used throughout the project.
pub mod utils;

/// Project paths and filesystem identifiers.
///
/// Defines standard names for configuration files, data directories,
/// and other environment-specific constants used for IO operations.
pub mod consts;

pub mod metadata;
/// Standard OS-specific filesystem paths for Knot artifacts.
pub mod paths;

/// The `errors` module provides definitions and utilities for handling
/// errors throughout the application. This may include custom error
/// types, conversions, and error-related functionality tailored to
/// the application's domain.
///
/// Typical usage:
/// - Define and manage application-specific errors.
/// - Implement `std::error::Error` and `std::fmt::Display` for custom error types.
/// - Facilitate consistent error handling and propagation.
///
/// Note:
/// Ensure errors defined in this module are meaningful and provide
/// clear context when surfaced to aid debugging and maintenance.
pub mod errors;
