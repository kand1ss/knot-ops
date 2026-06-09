mod builder;
mod stage;
pub use builder::*;
pub use stage::*;

use crate::ErrorReport;
use crate::renderer::{TaskRenderer, TaskRenderingStrategy};
use std::{borrow::Cow, collections::HashSet, time::Instant};

pub(crate) enum TaskEntry<'a> {
    Stage(Stage<'a>),
    Extra(TaskRenderer<'a>),
}

impl<'a> TaskEntry<'a> {
    #[cfg(test)]
    pub(crate) fn unwrap_stage(&self) -> &Stage<'a> {
        if let TaskEntry::Stage(stage) = self {
            stage
        } else {
            panic!("Called unwrap_stage on non-Stage TaskEntry")
        }
    }

    #[cfg(test)]
    pub(crate) fn unwrap_extra(&self) -> &TaskRenderer<'a> {
        if let TaskEntry::Extra(extra) = self {
            extra
        } else {
            panic!("Called unwrap_extra on non-extra TaskEntry")
        }
    }
}

/// A high-level representation of a complex, multi-step operation.
///
/// A `Task` acts as an orchestrator for a collection of individual [`Stage`]s.
/// It manages the overall state of the operation, automatically determining whether
/// the entire task is running, completed successfully, or failed based on the
/// statuses of its child stages.
///
/// The `Task` utilizes a terminal progress bar (`pb`) to render its current
/// overarching status to the user.
///
/// # State Transitions
///
/// * The task completes successfully only when **all** stages are marked as finished
///   (either via `ok` or `skip`).
/// * The task fails immediately if **any** stage is marked as failed.
pub struct Task<'a> {
    pub(crate) renderer: TaskRenderer<'a>,
    pub(crate) entries: Vec<TaskEntry<'a>>,
    start_time: Instant,
    finished_stages: HashSet<Cow<'a, str>>,
    failed_stages: HashSet<Cow<'a, str>>,
    is_finished: bool,
}

impl<'a> Task<'a> {
    /// Initializes a new `Task` with a given name, root progress bar, and a list of stages.
    ///
    /// Upon creation, the task is marked with a "Waiting" or "Title" status in the UI.
    ///
    /// # Arguments
    ///
    /// * `name` - The display name of the task.
    /// * `pb` - The root [`ProgressBar`] used to display the task's main status.
    /// * `stages` - A vector of [`Stage`] instances that belong to this task.
    pub(crate) fn new(renderer: TaskRenderer<'a>, entries: Vec<TaskEntry<'a>>) -> Self {
        Self {
            renderer,
            entries,
            start_time: Instant::now(),
            finished_stages: HashSet::new(),
            failed_stages: HashSet::new(),
            is_finished: false,
        }
    }

    pub(crate) fn clear_children(&mut self) {
        let mut nodes_to_clear = Vec::new();
        for entry in self.entries.iter_mut() {
            match entry {
                TaskEntry::Stage(stage) => {
                    nodes_to_clear.extend(stage.renderer.clear());
                }
                TaskEntry::Extra(renderer) => {
                    nodes_to_clear.extend(renderer.clear());
                }
            }
        }

        if let TaskRenderingStrategy::Interactive(main_node, layout, _) = &self.renderer.strategy {
            layout.update_anchor_if_in(&nodes_to_clear, main_node.clone());
        }

        self.entries.clear();
    }

    pub(crate) fn finish_children(&mut self) {
        for entry in self.entries.iter() {
            match entry {
                TaskEntry::Stage(stage) => stage.renderer.finish_silently(),
                TaskEntry::Extra(renderer) => renderer.finish_silently(),
            }
        }
    }

    /// Sets the overarching task as successfully completed.
    ///
    /// This also automatically clears the UI of all child subtasks to keep
    /// the terminal output clean.
    pub(crate) fn set_completed(&mut self, msg: &str) {
        if self.is_finished {
            return;
        }

        self.renderer.success(msg, self.start_time);
        self.is_finished = true;
        self.clear_children();
    }

    pub(crate) fn set_failed_and_insert(&mut self, error: impl Into<ErrorReport>) {
        self.renderer.fail_insert(error, self.start_time);
        self.is_finished = true;
    }

    /// Sets the overarching task as failed.
    pub(crate) fn set_failed_and_freeze(&mut self) {
        self.renderer.fail(self.start_time);
        self.is_finished = true;
        self.finish_children();
    }

    pub(crate) fn set_failed_and_clear(&mut self, error: impl Into<ErrorReport>) {
        self.set_failed_and_insert(error);
        self.clear_children();
    }

    pub(crate) fn stage_iterator(&self) -> impl Iterator<Item = &Stage<'a>> {
        self.entries.iter().filter_map(|e| {
            if let TaskEntry::Stage(s) = e {
                Some(s)
            } else {
                None
            }
        })
    }

    /// Re-evaluates the overarching task state based on the states of its stages.
    ///
    /// This is called internally after any stage changes state. If a stage failed,
    /// the entire task is failed. If all stages are finished, the task is completed.
    fn update_state(&mut self) -> bool {
        if self.is_finished {
            return false;
        }

        if !self.failed_stages.is_empty() {
            self.set_failed_and_freeze();
            return true;
        }

        let all_finished = self
            .stage_iterator()
            .all(|k| self.finished_stages.contains(&k.id));
        if all_finished {
            self.set_completed("success");
        }
        true
    }

    /// Retrieves a reference to a specific stage by its ID.   
    fn get_stage(&self, id: &str) -> Option<&Stage<'_>> {
        self.entries.iter().find_map(|s| {
            if let TaskEntry::Stage(stage) = s
                && stage.id == id
            {
                return Some(stage);
            }
            None
        })
    }

    fn get_stage_mut(&mut self, id: &str) -> Option<&mut Stage<'a>> {
        self.entries.iter_mut().find_map(move |s| {
            if let TaskEntry::Stage(stage) = s
                && stage.id == id
            {
                return Some(stage);
            }
            None
        })
    }

    /// Transitions a specific stage to the "Running" state.
    ///
    /// This resets any previous failure or finished states for this stage.
    ///
    /// # Returns
    ///
    /// Returns `true` if the stage was found and updated, or `false` if the ID is invalid.
    pub fn run_by_id(&mut self, id: &str, msg: Option<&str>) -> bool {
        {
            let Some(stage) = self.get_stage(id) else {
                return false;
            };
            let mut start = stage.start_time.lock().unwrap();
            *start = Some(Instant::now());
            stage.run(msg);
        }

        self.finished_stages.remove(id);
        self.failed_stages.remove(id);

        self.update_state()
    }

    /// Marks a specific stage as successfully completed.
    ///
    /// # Returns
    ///
    /// Returns `true` if the stage was found and updated, or `false` if the ID is invalid.
    pub fn ok_by_id<E, T>(&mut self, id: E, msg: T) -> bool
    where
        E: Into<Cow<'a, str>>,
        T: Into<Cow<'a, str>>,
    {
        let id = id.into();
        let msg = msg.into();
        {
            let Some(stage) = self.get_stage(&id) else {
                return false;
            };
            stage.success(&msg);
        }

        self.finished_stages.insert(id.clone());
        self.failed_stages.remove(&id);

        self.update_state()
    }

    /// Marks a specific stage as failed.
    ///
    /// This will subsequently cause the overarching [`Task`] to transition into
    /// a failed state as well.
    ///
    /// # Returns
    ///
    /// Returns `true` if the stage was found and updated, or `false` if the ID is invalid.
    pub fn fail_by_id<T>(&mut self, id: T, report: impl Into<ErrorReport>) -> bool
    where
        T: Into<Cow<'a, str>>,
    {
        let id = id.into();
        let report = report.into();
        {
            let Some(stage) = self.get_stage_mut(&id) else {
                return false;
            };
            stage.fail(report);
        }

        self.finished_stages.remove(&id);
        self.failed_stages.insert(id.clone());

        self.update_state()
    }

    /// Marks a specific stage as skipped.
    ///
    /// Skipped stages count as "finished" towards the completion of the overarching task.
    ///
    /// # Returns
    ///
    /// Returns `true` if the stage was found and updated, or `false` if the ID is invalid.
    pub fn skip_by_id<E, T>(&mut self, id: E, msg: T) -> bool
    where
        E: Into<Cow<'a, str>>,
        T: Into<Cow<'a, str>>,
    {
        let id = id.into();
        let msg = msg.into();
        {
            let Some(stage) = self.get_stage(&id) else {
                return false;
            };
            stage.skip(&msg);
        }

        self.finished_stages.insert(id.clone());
        self.failed_stages.remove(&id);

        self.update_state()
    }

    fn collect_stage_ids(&self) -> Vec<String> {
        self.stage_iterator()
            .map(|s| s.id.clone().into_owned())
            .collect()
    }

    pub(crate) fn perform_ok<T>(&mut self, msg: T)
    where
        T: Into<Cow<'a, str>> + Clone,
    {
        if self.is_finished {
            return;
        }
        let msg = msg.into();
        let ids = self.collect_stage_ids();
        if ids.is_empty() {
            self.set_completed(&msg);
        } else {
            for id in ids {
                self.ok_by_id(id, msg.clone());
            }
        }
    }

    /// Performs a bulk operation: marks all stages as completed.
    pub fn ok<T>(mut self, msg: T)
    where
        T: Into<Cow<'a, str>> + Clone,
    {
        self.perform_ok(msg);
    }

    pub(crate) fn perform_fail(&mut self, error: impl Into<ErrorReport>) {
        if self.is_finished {
            return;
        }
        let error = error.into();
        let ids = self.collect_stage_ids();
        for id in ids {
            self.fail_by_id(id, error.clone());
        }
        self.set_failed_and_clear(error);
    }

    /// Performs a bulk operation: marks all stages as failed.
    pub fn fail(mut self, error: impl Into<ErrorReport>) {
        self.perform_fail(error);
    }

    pub(crate) fn perform_run(&mut self, msg: Option<&str>) {
        if self.is_finished {
            return;
        }
        let ids = self.collect_stage_ids();
        for id in ids {
            self.run_by_id(&id, msg);
        }
    }

    /// Performs a bulk operation: marks all stages as running.
    pub fn run(&mut self, msg: Option<&str>) {
        self.perform_run(msg);
    }

    pub(crate) fn perform_skip(&mut self, msg: &'a str) {
        if self.is_finished {
            return;
        }
        let ids = self.collect_stage_ids();
        for id in ids {
            self.skip_by_id(id, msg);
        }
    }

    /// Performs a bulk operation: marks all stages as skipped.
    pub fn skip(mut self, msg: &'a str) {
        self.perform_skip(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layout::{LayoutNode, TaskLayout},
        renderer::TaskRenderingStrategy,
        style::TaskStyle,
        test_utils::hidden_layout,
    };
    use indicatif::ProgressBar;
    use std::sync::Arc;

    fn hidden_pb(layout: &TaskLayout) -> LayoutNode {
        layout.insert(ProgressBar::hidden())
    }

    fn create_stage<'a>(
        layout: Arc<TaskLayout>,
        id: &'a str,
        style: Arc<TaskStyle<'a>>,
    ) -> Stage<'a> {
        let stage_node = hidden_pb(&layout);

        let renderer = TaskRenderer::new(
            format!("Stage {}", id),
            crate::renderer::TaskRenderingStrategy::Interactive(
                stage_node,
                Arc::clone(&layout),
                vec![],
            ),
            Arc::clone(&style),
            2,
        );

        Stage::new(id.to_string(), renderer)
    }

    fn create_stage_entry<'a>(
        id: &'a str,
        layout: Arc<TaskLayout>,
        style: Arc<TaskStyle<'a>>,
    ) -> TaskEntry<'a> {
        TaskEntry::Stage(create_stage(layout, id, style))
    }

    fn create_test_task<'a>(layout: Arc<TaskLayout>) -> Task<'a> {
        let style = Arc::new(TaskStyle::modern());

        let stages = vec![
            create_stage_entry("stage_1", Arc::clone(&layout), Arc::clone(&style)),
            create_stage_entry("stage_2", Arc::clone(&layout), Arc::clone(&style)),
            create_stage_entry("stage_3", Arc::clone(&layout), Arc::clone(&style)),
        ];

        let head_node = hidden_pb(&layout);
        let head_renderer = TaskRenderer::new(
            "Test Task",
            crate::renderer::TaskRenderingStrategy::Interactive(
                head_node,
                Arc::clone(&layout),
                vec![],
            ),
            Arc::clone(&style),
            0,
        );

        Task::new(head_renderer, stages)
    }

    #[test]
    fn test_task_initialization() {
        let task = create_test_task(hidden_layout());

        assert_eq!(task.renderer.name, "Test Task");
        assert_eq!(task.entries.len(), 3);
        assert!(task.finished_stages.is_empty());
        assert!(task.failed_stages.is_empty());
    }

    #[test]
    fn test_stage_initialization() {
        let style = Arc::new(TaskStyle::modern());
        let stage = create_stage(hidden_layout(), "test_id", style);

        assert_eq!(stage.id, "test_id");
        assert_eq!(stage.renderer.name, "Stage test_id");
        assert!(
            stage.start_time.lock().unwrap().is_none(),
            "Start time should be None initially"
        );
    }

    #[test]
    fn test_invalid_id_returns_false() {
        let mut task = create_test_task(hidden_layout());

        assert!(!task.run_by_id("invalid_id", None));
        assert!(!task.ok_by_id("invalid_id", "done"));
        assert!(!task.fail_by_id("invalid_id", "error"));
        assert!(!task.skip_by_id("invalid_id", "skip"));
    }

    #[test]
    fn test_run_by_id_updates_state() {
        let mut task = create_test_task(hidden_layout());

        let success = task.run_by_id("stage_1", Some("running"));
        assert!(success);

        let stage = task.get_stage("stage_1").unwrap();
        assert!(stage.start_time.lock().unwrap().is_some());
    }

    #[test]
    fn test_ok_by_id_adds_to_finished() {
        let mut task = create_test_task(hidden_layout());

        assert!(task.ok_by_id("stage_1", "success"));
        assert!(task.finished_stages.contains("stage_1"));
        assert!(!task.failed_stages.contains("stage_1"));
    }

    #[test]
    fn test_fail_by_id_adds_to_failed() {
        let mut task = create_test_task(hidden_layout());

        assert!(task.fail_by_id("stage_2", "error"));
        assert!(task.failed_stages.contains("stage_2"));
        assert!(!task.finished_stages.contains("stage_2"));
    }

    #[test]
    fn test_skip_by_id_counts_as_finished() {
        let mut task = create_test_task(hidden_layout());

        assert!(task.skip_by_id("stage_3", "skipped"));
        assert!(task.finished_stages.contains("stage_3"));
        assert!(!task.failed_stages.contains("stage_3"));
    }

    #[test]
    fn test_task_completes_when_all_stages_finished() {
        let mut task = create_test_task(hidden_layout());

        task.ok_by_id("stage_1", "ok");
        task.ok_by_id("stage_2", "ok");

        assert_eq!(task.finished_stages.len(), 2);

        task.ok_by_id("stage_3", "ok");
        assert_eq!(task.finished_stages.len(), 3);
        assert!(task.failed_stages.is_empty());
    }

    #[test]
    fn test_task_completes_with_mixed_ok_and_skip() {
        let mut task = create_test_task(hidden_layout());

        task.ok_by_id("stage_1", "ok");
        task.skip_by_id("stage_2", "skipped");
        task.ok_by_id("stage_3", "ok");

        assert_eq!(task.finished_stages.len(), 3);
    }

    #[test]
    fn test_task_fails_if_any_stage_fails() {
        let mut task = create_test_task(hidden_layout());

        task.ok_by_id("stage_1", "ok");
        task.fail_by_id("stage_2", "error");
        assert!(task.failed_stages.contains("stage_2"));
    }

    #[test]
    fn test_bulk_ok() {
        let mut task = create_test_task(hidden_layout());

        task.perform_ok("all done");

        assert_eq!(task.finished_stages.len(), 3);
        assert!(task.failed_stages.is_empty());
    }

    #[test]
    fn test_bulk_fail() {
        let mut task = create_test_task(hidden_layout());
        task.perform_fail("all failed");

        assert_eq!(task.failed_stages.len(), 3);
        assert!(task.finished_stages.is_empty());
    }

    #[test]
    fn test_bulk_skip() {
        let mut task = create_test_task(hidden_layout());
        task.perform_skip("all skipped");
        assert_eq!(task.finished_stages.len(), 3);
    }

    #[test]
    fn test_bulk_run() {
        let mut task = create_test_task(hidden_layout());

        task.perform_run(Some("running everywhere"));

        assert!(task.finished_stages.is_empty());
        assert!(task.failed_stages.is_empty());

        for entry in &task.entries {
            if let TaskEntry::Stage(stage) = entry {
                assert!(stage.start_time.lock().unwrap().is_some());
            } else {
                panic!("Wrong entry");
            }
        }
    }

    #[test]
    fn test_single_task_without_stages_completion() {
        let layout = hidden_layout();
        let style = Arc::new(TaskStyle::modern());
        let renderer = TaskRenderer::new(
            "Single Task",
            TaskRenderingStrategy::Interactive(hidden_pb(&layout), layout, vec![]),
            style,
            0,
        );
        let mut task = Task::new(renderer, vec![]);

        task.perform_ok("done");
        assert!(task.finished_stages.is_empty());
        assert!(task.failed_stages.is_empty());
        assert!(task.entries.is_empty());
    }

    #[test]
    fn test_single_task_without_stages_failure() {
        let layout = hidden_layout();
        let style = Arc::new(TaskStyle::modern());
        let renderer = TaskRenderer::new(
            "Single Task",
            TaskRenderingStrategy::Interactive(hidden_pb(&layout), layout, vec![]),
            style,
            0,
        );
        let mut task = Task::new(renderer, vec![]);

        task.perform_fail("fail");
        assert!(task.finished_stages.is_empty());
        assert!(task.failed_stages.is_empty());
    }
}
