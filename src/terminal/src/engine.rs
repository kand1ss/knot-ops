use crate::layout::anchor::TaskLayoutAnchor;
use crate::layout::{LayoutNode, TaskLayout};
use crate::renderer::{PlainWriter, RenderMode, TaskRenderer, TaskRenderingStrategy};
use crate::style::TaskStyle;
use crate::{Step, TaskBuilder, TaskSequenceBuilder};
use indicatif::{MultiProgress, ProgressBar};
use std::io::IsTerminal;
use std::sync::Arc;

/// The central coordinator for rendering progress outputs.
///
/// `TaskEngine` detects terminal capabilities (interactive TTY vs. non-interactive/CI)
/// and manages active rendering modes and styles. It acts as a factory for
/// [`Step`], [`TaskBuilder`], and [`TaskSequenceBuilder`].
#[derive(Clone)]
pub struct TaskEngine<'a> {
    style: Arc<TaskStyle<'a>>,
    mode: Arc<RenderMode>,
}

impl<'a> TaskEngine<'a> {
    fn detect_mode(multi: Option<MultiProgress>, anchor_node: Option<LayoutNode>) -> RenderMode {
        if std::env::var("CI").is_ok() || !std::io::stdout().is_terminal() {
            RenderMode::plain_stdout()
        } else {
            let multi = multi.unwrap_or_default();
            let anchor = TaskLayoutAnchor::new(anchor_node);
            RenderMode::Interactive(Arc::new(TaskLayout::new(multi, anchor)))
        }
    }

    fn detect_style(mode: &RenderMode) -> TaskStyle<'a> {
        match mode {
            RenderMode::Interactive(..) => TaskStyle::modern(),
            RenderMode::Plain(..) => TaskStyle::plain(),
        }
    }

    fn inner_new(multi: Option<MultiProgress>, anchor: Option<LayoutNode>) -> Self {
        let mode = Self::detect_mode(multi, anchor);
        let style = Arc::new(Self::detect_style(&mode));
        Self {
            style,
            mode: Arc::new(mode),
        }
    }

    /// Creates a new `TaskEngine` using a custom [`MultiProgress`] instance.
    pub fn with_multi(multi: MultiProgress) -> Self {
        Self::inner_new(Some(multi), None)
    }

    /// Creates a new `TaskEngine`.
    ///
    /// This method automatically detects if stdout is a TTY and whether it is running in a CI.
    pub fn new() -> Self {
        Self::inner_new(None, None)
    }

    /// Forces a specific [`RenderMode`] (e.g. `RenderMode::Plain`).
    pub fn with_render_mode(mut self, mode: RenderMode) -> Self {
        self.style = Arc::new(Self::detect_style(&mode));
        self.mode = Arc::new(mode);
        self
    }

    /// Configures a custom [`PlainWriter`] implementation to use when in plain rendering mode.
    pub fn with_plain_writer(mut self, writer: impl PlainWriter + 'static) -> Self {
        if let RenderMode::Plain(_) = *self.mode {
            self.mode = Arc::new(RenderMode::Plain(Arc::new(writer)));
        }
        self
    }

    /// Configures custom style prefixes and frames via [`TaskStyle`].
    pub fn with_style(mut self, style: TaskStyle<'a>) -> Self {
        self.style = Arc::new(style);
        self
    }

    /// Inserts a blank spacing line in the terminal (interactive mode only).
    pub fn space(&self) {
        if let RenderMode::Interactive(layout) = &*self.mode {
            let spacer = crate::utils::create_space();
            layout.insert(spacer.clone());
            spacer.finish_with_message(" ");
        }
    }

    /// Creates a new standalone [`Step`].
    ///
    /// # Arguments
    ///
    /// * `name` - The display name of the step.
    /// * `auto_run` - If `true`, the step is instantly marked as running.
    pub fn step(&self, name: &'a str, auto_run: bool) -> Step<'a> {
        let strategy = match &*self.mode {
            RenderMode::Interactive(layout) => {
                let node = layout.insert(ProgressBar::new(0));
                TaskRenderingStrategy::interactive(node, Arc::clone(layout))
            }
            RenderMode::Plain(writer) => TaskRenderingStrategy::plain("", Arc::clone(writer)),
        };

        let renderer = TaskRenderer::new(name, strategy, Arc::clone(&self.style), 0);
        let mut step = Step::new(renderer);
        if auto_run {
            step.run(None);
        }
        step
    }

    /// Creates a new [`TaskBuilder`] for orchestrating a task with multiple stages.
    pub fn task(&self, name: &'a str) -> TaskBuilder<'a> {
        TaskBuilder::new(name, Arc::clone(&self.style), Arc::clone(&self.mode))
    }

    /// Creates a new [`TaskSequenceBuilder`] for running stages strictly one after another.
    pub fn sequence(&self, name: &'a str) -> TaskSequenceBuilder<'a> {
        TaskSequenceBuilder::new(self.task(name))
    }
}
impl<'a> Default for TaskEngine<'a> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::InMemoryTerm;
    use indicatif::ProgressDrawTarget;

    #[test]
    fn test_rendering_bug_with_sequence_and_space() {
        let term = InMemoryTerm::new(100, 50);
        let multi =
            MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(term.clone())));
        let engine = TaskEngine::new().with_render_mode(RenderMode::Interactive(Arc::new(
            TaskLayout::new(multi, TaskLayoutAnchor::new(None)),
        )));

        let mut seq = engine
            .sequence("Sequence Task")
            .with_stage("Stage 1")
            .build(false);
        seq.ok("Stage 1 done");
        seq.finish("Seq done");

        engine.space();

        let mut empty_task = engine.task("Empty Task").start(false);
        empty_task.set_completed("Empty done");

        let content = term.contents();
        let empty_task_count = content.matches("Empty Task").count();
        let stage_1_count = content.matches("Stage 1").count();
        let seq_task_count = content.matches("Sequence Task").count();

        println!("Terminal output:\n{}", content);

        assert_eq!(
            empty_task_count, 1,
            "Empty Task should appear exactly once, but appeared {} times.",
            empty_task_count
        );
        assert_eq!(
            stage_1_count, 0,
            "Stage 1 should be cleared, but appeared {} times.",
            stage_1_count
        );
        assert_eq!(
            seq_task_count, 1,
            "Sequence Task should appear exactly once, but appeared {} times.",
            seq_task_count
        );
    }
}
