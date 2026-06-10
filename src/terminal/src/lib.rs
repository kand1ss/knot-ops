//! A concurrent progress reporting and task orchestration library.
//!
//! `knot-terminal` provides a structured, high-level API to output progress of
//! complex steps, tasks, and task sequences to a console.
//!
//! It supports both interactive modes (fancy live spinners and progress bars that update in-place)
//! and plain mode (plain output optimized for non-TTY streams and CI environments).
//!
//! # Primitives
//! - [`TaskEngine`]: The central coordinator that auto-detects standard stream characteristics
//!   and configures style presets.
//! - [`Step`]: A single standalone progress unit.
//! - [`Task`]: A compound operation consisting of multiple sequential or parallel [`Stage`] subtasks.
//! - [`TaskSequence`]: A linear state machine enforcing sequential execution of stages.

mod engine;
mod error;
pub mod layout;
pub mod renderer;
mod sequence;
mod step;
pub mod style;
mod task;
pub mod test_utils;
pub(crate) mod utils;

pub use engine::*;
pub use error::*;
pub use sequence::*;
pub use step::*;
pub use task::*;
