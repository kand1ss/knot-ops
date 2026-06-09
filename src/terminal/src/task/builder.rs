use std::borrow::Cow;
use std::sync::Arc;

use crate::layout::LayoutNode;
use crate::renderer::{RenderMode, TaskRenderer, TaskRenderingStrategy};
use crate::style::TaskStyle;
use crate::utils::create_space;
use crate::{Stage, Task, TaskEntry};
use indicatif::ProgressBar;

enum BuilderStep<'a> {
    Stage {
        id: Cow<'a, str>,
        name: Cow<'a, str>,
        auto_run: bool,
    },
    Group(Option<&'a str>),
}

/// A builder for constructing and configuring a [`Task`] alongside its visual representation.
///
/// `TaskBuilder` utilizes the consuming builder pattern to provide a fluent interface
/// for defining the stages of a complex operation. It abstracts away the setup of progress bars,
/// ensuring that parent and child bars are linked and rendered in the correct order.
///
/// Users typically obtain a `TaskBuilder` by calling [`TaskEngine::task`](crate::TaskEngine::task).
///
/// # Examples
///
/// ```rust
/// use knot_terminal::TaskEngine;
///
/// let engine = TaskEngine::new();
///
/// // Construct a task with two stages, automatically starting the first one.
/// let mut task = engine.task("System Update")
///     .with_stage("download", "Downloading packages", true)
///     .with_stage("install", "Installing updates", false)
///     .start(false);
/// ```
pub struct TaskBuilder<'a> {
    name: Cow<'a, str>,
    steps: Vec<BuilderStep<'a>>,
    style: Arc<TaskStyle<'a>>,
    mode: Arc<RenderMode>,
}

impl<'a> TaskBuilder<'a> {
    /// Initializes a new TaskBuilder.
    ///
    /// This is an internal constructor. Users should typically create a builder
    /// via a higher-level orchestrator or sequence manager.
    pub(crate) fn new<T>(name: T, style: Arc<TaskStyle<'a>>, mode: Arc<RenderMode>) -> Self
    where
        T: Into<Cow<'a, str>>,
    {
        Self {
            name: name.into(),
            steps: Vec::new(),
            style,
            mode,
        }
    }

    /// Appends a new stage to the task definition.
    ///
    /// Stages are rendered in the terminal in the exact order they are added
    /// using this method.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique internal identifier for this stage.
    /// * `stage_name` - The human-readable name displayed in the UI.
    /// * `auto_run` - If set to `true`, this stage will automatically transition
    ///   to the "Running" state as soon as [`Self::start`] is called.
    ///
    /// # Returns
    ///
    /// Returns the consumed `Self` to allow for method chaining.
    pub fn with_stage<E, T>(mut self, id: E, stage_name: T, auto_run: bool) -> Self
    where
        E: Into<Cow<'a, str>>,
        T: Into<Cow<'a, str>>,
    {
        self.steps.push(BuilderStep::Stage {
            id: id.into(),
            name: stage_name.into(),
            auto_run,
        });
        self
    }

    /// Appends a separator group header to the task layout.
    ///
    /// Groups are used to visually divide stages of a task.
    ///
    /// # Arguments
    ///
    /// * `header` - An optional string title for the group separator line.
    pub fn with_group(mut self, header: Option<&'a str>) -> Self {
        self.steps.push(BuilderStep::Group(header));
        self
    }

    fn create_head_renderer(&self) -> (TaskRenderer<'a>, Option<LayoutNode>) {
        match &*self.mode {
            RenderMode::Plain(writer) => {
                let r = TaskRenderer::new(
                    self.name.clone(),
                    TaskRenderingStrategy::plain("", Arc::clone(writer)),
                    Arc::clone(&self.style),
                    0,
                );
                (r, None)
            }
            RenderMode::Interactive(layout) => {
                let head_bar = layout.insert(ProgressBar::new(0));
                let r = TaskRenderer::new(
                    self.name.clone(),
                    TaskRenderingStrategy::interactive(head_bar.clone(), Arc::clone(layout)),
                    Arc::clone(&self.style),
                    0,
                );
                (r, Some(head_bar))
            }
        }
    }

    fn create_child_renderer(
        &self,
        child_name: Cow<'a, str>,
        insertion_point: &mut Option<LayoutNode>,
        indent_spaces: usize,
    ) -> TaskRenderer<'a> {
        match &*self.mode {
            RenderMode::Plain(writer) => {
                let prefix = format!("[{}]", self.name);
                TaskRenderer::new(
                    child_name,
                    TaskRenderingStrategy::plain(prefix, Arc::clone(writer)),
                    Arc::clone(&self.style),
                    indent_spaces,
                )
            }
            RenderMode::Interactive(layout) => {
                let point = insertion_point.as_ref().unwrap();
                let bar = layout.insert_after(point, ProgressBar::new(0));

                *insertion_point = Some(bar.clone());

                TaskRenderer::new(
                    child_name,
                    TaskRenderingStrategy::interactive(bar, Arc::clone(layout)),
                    Arc::clone(&self.style),
                    indent_spaces,
                )
            }
        }
    }

    fn apply_auto_indent(
        &self,
        entries: &mut Vec<TaskEntry<'a>>,
        insertion_point: &mut Option<LayoutNode>,
        indent_spaces: usize,
    ) {
        if let RenderMode::Interactive(layout) = &*self.mode {
            let pb = create_space();
            let point = insertion_point.as_ref().unwrap();
            let spacer = layout.insert_after(point, pb);

            let renderer = TaskRenderer::new(
                "",
                TaskRenderingStrategy::interactive(spacer.clone(), Arc::clone(layout)),
                Arc::clone(&self.style),
                indent_spaces,
            );

            entries.push(TaskEntry::Extra(renderer));
            spacer.finish_with_message(" ");
            *insertion_point = Some(spacer);
        }
    }

    /// Finalizes the configuration, builds the UI components, and spawns the [`Task`].
    ///
    /// This method performs the following operations:
    /// 1. Instantiates the root [`ProgressBar`] (or uses the provided custom root).
    /// 2. Iterates through the defined stages, creating a linked progress bar for each,
    ///    ensuring they are physically drawn below the root bar in the terminal.
    /// 3. Constructs the [`Task`] and its child [`Stage`]s.
    /// 4. Automatically triggers the `run` state for any stages configured with `auto_run`.
    ///
    /// # Returns
    ///
    /// Returns the fully initialized and active [`Task`].
    pub fn start(mut self, auto_indent: bool) -> Task<'a> {
        let child_indent_spaces = 3;
        let mut entries = Vec::new();
        let mut autostart = Vec::new();

        let (head_renderer, mut insertion_point) = self.create_head_renderer();
        head_renderer.title();
        let steps = std::mem::take(&mut self.steps);

        for step in steps {
            match step {
                BuilderStep::Stage { id, name, auto_run } => {
                    let stage_renderer =
                        self.create_child_renderer(name, &mut insertion_point, child_indent_spaces);
                    stage_renderer.wait(None);

                    let stage = Stage::new(id.clone(), stage_renderer);
                    entries.push(TaskEntry::Stage(stage));

                    if auto_run {
                        autostart.push(id);
                    }
                }
                BuilderStep::Group(header) => {
                    let group_name = header.map(Cow::from).unwrap_or(Cow::Borrowed(""));
                    let group_renderer = self.create_child_renderer(
                        group_name,
                        &mut insertion_point,
                        child_indent_spaces,
                    );

                    group_renderer.separator();
                    entries.push(TaskEntry::Extra(group_renderer));
                }
            }
        }

        if auto_indent {
            self.apply_auto_indent(&mut entries, &mut insertion_point, child_indent_spaces);
        }

        if let RenderMode::Interactive(layout) = &*self.mode
            && let Some(point) = insertion_point
        {
            layout.set_anchor(point);
        }

        let mut task = Task::new(head_renderer, entries);
        for id in autostart {
            task.run_by_id(&id, None);
        }

        task
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::hidden_layout;
    use rstest::rstest;

    fn style(mode: &RenderMode) -> Arc<TaskStyle<'static>> {
        Arc::new(match mode {
            RenderMode::Interactive(..) => TaskStyle::modern(),
            RenderMode::Plain(..) => TaskStyle::plain(),
        })
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_with_stage_no_autorun(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Test Flow", style(&mode), Arc::new(mode))
            .with_stage("stage_1", "Download", false);

        let step = &builder.steps[0];
        assert_eq!(builder.steps.len(), 1);
        assert!(matches!(step, BuilderStep::Stage { .. }));
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_with_stage_autorun(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Test Flow", style(&mode), Arc::new(mode))
            .with_stage("stage_1", "Download", true);

        assert_eq!(builder.steps.len(), 1);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_start_creates_empty_task(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Empty Task", style(&mode), Arc::new(mode));

        let task = builder.start(false);
        assert!(task.entries.is_empty());
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_start_creates_task_with_correct_stages(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("App Build", style(&mode), Arc::new(mode))
            .with_stage("s1", "Step 1", false)
            .with_stage("s2", "Step 2", false);

        let task = builder.start(false);

        assert_eq!(task.entries.len(), 2);
        assert_eq!(task.entries[0].unwrap_stage().id, "s1");
        assert_eq!(task.entries[1].unwrap_stage().id, "s2");
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_start_applies_autorun_correctly(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Autostart Flow", style(&mode), Arc::new(mode))
            .with_stage("s1", "Manual Step", false)
            .with_stage("s2", "Auto Step", true);

        let task = builder.start(false);
        assert_eq!(task.entries.len(), 2);

        let stage1 = &task.entries[0];
        let stage2 = &task.entries[1];

        assert!(
            stage1.unwrap_stage().start_time.lock().unwrap().is_none(),
            "Stage 1 should not be running automatically"
        );

        assert!(
            stage2.unwrap_stage().start_time.lock().unwrap().is_some(),
            "Stage 2 should be running because of auto_run=true"
        );
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_exists(#[case] mode: RenderMode) {
        let builder =
            TaskBuilder::new("Group", style(&mode), Arc::new(mode)).with_group(Some("Header"));
        let task = builder.start(false);

        assert_eq!(task.entries.len(), 1);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_with_stage_on_the_start(#[case] mode: RenderMode) {
        let task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_group(None)
            .with_stage("s1", "STAGE 1", false)
            .start(false);

        assert_eq!(task.entries.len(), 2);
        task.entries[0].unwrap_extra();
        task.entries[1].unwrap_stage();
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_with_stage_on_the_middle(#[case] mode: RenderMode) {
        let task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_stage("s1", "STAGE 1", false)
            .with_group(None)
            .with_stage("s2", "STAGE 2", false)
            .start(false);

        assert_eq!(task.entries.len(), 3);
        task.entries[0].unwrap_stage();
        task.entries[1].unwrap_extra();
        task.entries[2].unwrap_stage();
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_with_stage_on_the_end(#[case] mode: RenderMode) {
        let task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_stage("s1", "STAGE 1", false)
            .with_stage("s2", "STAGE 2", false)
            .with_group(None)
            .start(false);

        assert_eq!(task.entries.len(), 3);
        task.entries[0].unwrap_stage();
        task.entries[1].unwrap_stage();
        task.entries[2].unwrap_extra();
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_with_stage_clears_when_finish(#[case] mode: RenderMode) {
        let mut task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_stage("s1", "STAGE 1", false)
            .with_group(None)
            .start(false);

        assert_eq!(task.entries.len(), 2);

        task.perform_ok("Finished");
        assert_eq!(task.entries.len(), 0);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_clears_when_finish(#[case] mode: RenderMode) {
        let mut task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_group(None)
            .start(false);

        assert_eq!(task.entries.len(), 1);

        task.perform_ok("Finished");
        assert_eq!(task.entries.len(), 0);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(hidden_layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_group_very_big_header_no_panic(#[case] mode: RenderMode) {
        let header = "X".repeat(9999);
        let task = TaskBuilder::new("Group with Stage", style(&mode), Arc::new(mode))
            .with_group(Some(&header))
            .start(false);

        task.ok("Finished");
    }
}
