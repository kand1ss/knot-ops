use crate::ErrorReport;
use crate::renderer::TaskRenderer;
use std::time::Instant;

/// Represents a standalone, single progress step.
///
/// A `Step` is used to track individual operations that do not have sub-stages.
/// It renders using a spinner in interactive mode.
pub struct Step<'a> {
    pub(crate) renderer: TaskRenderer<'a>,
    start: Option<Instant>,
}

impl<'a> Step<'a> {
    pub(crate) fn new(renderer: TaskRenderer<'a>) -> Self {
        renderer.wait(None);
        Self {
            renderer,
            start: None,
        }
    }

    fn time(&self) -> Instant {
        self.start.unwrap_or_else(Instant::now)
    }

    pub(crate) fn perform_run(&mut self, status: Option<&str>) {
        self.renderer.run(status);
        self.start = Some(Instant::now());
    }

    pub(crate) fn perform_ok(&self, status: &str) {
        let start = self.time();
        self.renderer.success(status, start);
    }

    pub(crate) fn perform_fail(&mut self, error: impl Into<ErrorReport>) {
        let error = error.into();
        let start = self.time();
        self.renderer.fail_insert(error, start);
    }

    pub(crate) fn perform_skip(&self, status: &str) {
        self.renderer.skip(status);
    }

    /// Transitions the step to the running state, displaying a spinner.
    ///
    /// # Arguments
    ///
    /// * `status` - An optional message explaining what the step is currently doing.
    pub fn run(&mut self, status: Option<&str>) {
        self.perform_run(status);
    }

    /// Successfully completes the step.
    ///
    /// # Arguments
    ///
    /// * `status` - The final success status message.
    pub fn ok(self, status: &str) {
        self.perform_ok(status);
    }

    /// Fails the step and registers an [`ErrorReport`].
    ///
    /// # Arguments
    ///
    /// * `error` - The detailed error or diagnostic details.
    pub fn fail(mut self, error: impl Into<ErrorReport>) {
        self.perform_fail(error);
    }

    /// Skips the step.
    ///
    /// # Arguments
    ///
    /// * `status` - A message detailing why the step was skipped.
    pub fn skip(self, status: &str) {
        self.perform_skip(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layout::{LayoutNode, TaskLayout, anchor::TaskLayoutAnchor},
        style::TaskStyle,
    };
    use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget};
    use std::sync::Arc;

    fn anchor() -> TaskLayoutAnchor {
        TaskLayoutAnchor::new(None)
    }

    fn hidden_multi() -> MultiProgress {
        MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
    }

    fn layout() -> Arc<TaskLayout> {
        Arc::new(TaskLayout::new(hidden_multi(), anchor()))
    }

    fn hidden_pb(layout: &TaskLayout) -> LayoutNode {
        layout.insert(ProgressBar::hidden())
    }

    fn create_step<'a>() -> Step<'a> {
        let layout = layout();
        let style = Arc::new(TaskStyle::modern());
        let stage_node = hidden_pb(&layout);

        let renderer = TaskRenderer::new(
            "Test Task",
            crate::renderer::TaskRenderingStrategy::Interactive(
                stage_node,
                Arc::clone(&layout),
                vec![],
            ),
            Arc::clone(&style),
            0,
        );

        Step::new(renderer)
    }

    #[test]
    fn test_step_initialization() {
        let step = create_step();

        assert_eq!(step.renderer.name, "Test Task");
        assert!(step.start.is_none());
    }

    #[test]
    fn test_time_fallback_without_run() {
        let step = create_step();

        let time_before = Instant::now();
        let step_time = step.time();
        let time_after = Instant::now();

        assert!(step_time >= time_before && step_time <= time_after);
    }

    #[test]
    fn test_run_sets_start_time() {
        let mut step = create_step();

        assert!(step.start.is_none());

        step.perform_run(Some("working"));
        assert!(step.start.is_some());
    }
}
