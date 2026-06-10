use crate::{TaskBuilder, TaskSequence};
use std::sync::{Arc, atomic::AtomicUsize};

/// A builder for constructing a task execution sequence.
///
/// `TaskSequenceBuilder` utilizes the consuming builder pattern to safely
/// formulate a list of stages. It guarantees that stages are registered
/// in the correct order before their actual execution begins.
///
/// Users typically obtain a `TaskSequenceBuilder` by calling [`TaskEngine::sequence`](crate::TaskEngine::sequence).
///
/// # Example
///
/// ```rust
/// use knot_terminal::TaskEngine;
///
/// let engine = TaskEngine::new();
/// let mut sequence = engine.sequence("Environment Deployment")
///     .with_stage("Pulling images")
///     .with_stage("Starting containers")
///     .with_stage("Healthcheck")
///     .start(false);
/// ```
/// After calling `start()`, the first stage ("Pulling images") begins automatically.
pub struct TaskSequenceBuilder<'a> {
    inner: TaskBuilder<'a>,
    stage_counter: Arc<AtomicUsize>,
}

impl<'a> TaskSequenceBuilder<'a> {
    /// Creates a new builder instance.
    pub(crate) fn new(builder: TaskBuilder<'a>) -> Self {
        Self {
            inner: builder,
            stage_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Adds a new sequential stage to the task chain.
    ///
    /// The internal stage identifier (ID) is generated automatically. This method
    /// takes ownership of `self` and returns the updated instance, enabling method chaining.
    pub fn with_stage(mut self, stage_name: &'a str) -> Self {
        let id = self
            .stage_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let new_inner = self
            .inner
            .with_stage(format!("stage_{}", id), stage_name, false);

        self.inner = new_inner;
        self
    }

    /// Appends a separator group header to the sequence layout.
    ///
    /// # Arguments
    ///
    /// * `header` - An optional string title for the group separator line.
    pub fn with_group(mut self, header: Option<&'a str>) -> Self {
        let new_inner = self.inner.with_group(header);
        self.inner = new_inner;
        self
    }

    /// Finalizes the configuration and starts the task sequence.
    ///
    /// This method creates and returns a [`TaskSequence`] controller, and
    /// **automatically activates the first stage** (transitioning it to "Running").
    pub fn start(self, auto_indent: bool) -> TaskSequence<'a> {
        let task = self.inner.start(auto_indent);
        TaskSequence::new(task, true)
    }

    /// Finalizes the configuration and returns a [`TaskSequence`] controller without starting it.
    pub fn build(self, auto_indent: bool) -> TaskSequence<'a> {
        let task = self.inner.start(auto_indent);
        TaskSequence::new(task, false)
    }
}
