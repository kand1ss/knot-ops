use crate::{ErrorReport, renderer::TaskRenderer};
use std::{borrow::Cow, sync::Mutex, time::Instant};

/// A single, indivisible step within a larger [`Task`].
///
/// A `Stage` represents a concrete action (e.g., "Downloading file", "Compiling asset").
/// It maintains its own progress bar and tracks the time elapsed since it started running.
/// Stages are primarily manipulated indirectly through their parent [`Task`].
pub struct Stage<'a> {
    /// The unique identifier used to target this stage.
    pub(crate) id: Cow<'a, str>,
    pub(crate) renderer: TaskRenderer<'a>,
    pub(crate) start_time: Mutex<Option<Instant>>,
}
impl<'a> Stage<'a> {
    /// Creates a new `Stage` and initializes its UI to a "Waiting" state.
    pub(crate) fn new<E>(id: E, renderer: TaskRenderer<'a>) -> Self
    where
        E: Into<Cow<'a, str>> + Clone,
    {
        let id = id.into();

        Self {
            id,
            renderer,
            start_time: Mutex::new(None),
        }
    }

    /// Updates the UI to show the stage is actively running.
    pub fn run(&self, msg: Option<&str>) {
        self.renderer.run(msg);
    }

    /// Updates the UI to show the stage is waiting to start.
    pub fn wait(&self, msg: Option<&str>) {
        self.renderer.wait(msg);
    }

    /// Updates the UI to show the stage has completed successfully.
    ///
    /// This method calculates the elapsed time since `mark_running` was called
    /// and displays it alongside the completion message.
    pub fn success(&self, msg: &str) {
        let start = self.start_time.lock().unwrap().unwrap_or_else(Instant::now);
        self.renderer.success(msg, start);
    }

    /// Updates the UI to show the stage has failed.
    pub fn fail(&mut self, error: impl Into<ErrorReport>) {
        let start = self.start_time.lock().unwrap().unwrap_or_else(Instant::now);
        self.renderer.fail_insert(error, start);
    }

    /// Updates the UI to show the stage was intentionally skipped.
    pub fn skip(&self, msg: &str) {
        self.renderer.skip(msg);
    }
}
