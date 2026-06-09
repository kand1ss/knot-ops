mod builder;
pub use builder::*;

use crate::{ErrorReport, Task, TaskEntry};
use std::borrow::Cow;

/// A state machine managing the sequential execution of stages.
///
/// `TaskSequence` ensures that stages are executed strictly one after another.
/// The controller provides safe state management: if a task is interrupted by an
/// error ([Self::fail]) or finished early ([Self::finish]), all remaining
/// stages are correctly marked as skipped.
///
/// Any method calls made after all stages have been exhausted are safely ignored,
/// preventing out-of-bounds panics.
pub struct TaskSequence<'a> {
    inner: Task<'a>,
    current_stage: usize,
    all_ids: Vec<Cow<'a, str>>,
}

impl<'a> TaskSequence<'a> {
    /// Activates the current stage, updating its visual representation to "In Progress".
    fn activate_current_stage(&mut self) {
        if let Some(id) = self.all_ids.get(self.current_stage) {
            self.inner.run_by_id(id, None);
        }
    }

    /// Initializes a new `TaskSequence` based on a prepared task.
    ///
    /// # Arguments
    ///
    /// * `task` - The underlying task containing the stages.
    /// * `autostart` - If `true`, the first stage is automatically transitioned to "Running".
    pub fn new(task: Task<'a>, autostart: bool) -> Self {
        let all_ids: Vec<Cow<'a, str>> = task
            .entries
            .iter()
            .filter_map(|e| {
                if let TaskEntry::Stage(s) = e {
                    Some(s.id.clone())
                } else {
                    None
                }
            })
            .collect();

        let mut sequence = Self {
            inner: task,
            current_stage: 0,
            all_ids,
        };
        if autostart {
            sequence.activate_current_stage();
        }
        sequence
    }

    /// Successfully completes the current stage and advances to the next one.
    ///
    /// If this was the last stage in the chain, the root task will also be
    /// marked as successfully completed.
    ///
    /// # Arguments
    ///
    /// * `result_msg` - The message to display next to the completed stage.
    pub fn ok(&mut self, result_msg: &'a str) {
        if let Some(id) = self.all_ids.get(self.current_stage) {
            self.inner.ok_by_id(id.clone(), result_msg);
            self.current_stage += 1;

            if self.current_stage < self.all_ids.len() {
                self.activate_current_stage();
            } else {
                self.inner.set_completed("success");
            }
        }
    }

    /// Marks the current stage as failed.
    ///
    /// Calling this method interrupts the normal execution flow. All remaining
    /// unexecuted stages are marked as "skipped", and the root task is
    /// transitioned to an "Error" status.
    ///
    /// # Arguments
    ///
    /// * `result_msg` - The error text to display in the terminal.
    pub fn fail(&mut self, report: impl Into<ErrorReport>) {
        if let Some(id) = self.all_ids.get(self.current_stage) {
            self.inner.fail_by_id(id.clone(), report);
            self.current_stage += 1;

            if self.current_stage < self.all_ids.len() {
                self.skip_stages(self.current_stage, "skipped due to previous error");
            }
        }
    }

    /// Skips the current stage without an error and advances to the next one.
    ///
    /// Used when an execution step is not required (e.g., the cache is up-to-date
    /// or a dependency is already met). If the last stage is skipped, the entire
    /// task is considered successfully completed.
    ///
    /// # Arguments
    ///
    /// * `result_msg` - A message explaining the reason for skipping (e.g., "cached").
    pub fn skip(&mut self, result_msg: &'a str) {
        if let Some(id) = self.all_ids.get(self.current_stage) {
            self.inner.skip_by_id(id.clone(), result_msg);
            self.current_stage += 1;

            if self.current_stage < self.all_ids.len() {
                self.activate_current_stage();
            } else {
                self.inner.set_completed("success");
            }
        }
    }

    /// A helper method to bulk-skip all remaining stages.
    fn skip_stages(&mut self, start_from: usize, msg: &'a str) {
        for id in &self.all_ids[start_from..] {
            self.inner.skip_by_id(id.clone(), msg);
        }
        self.current_stage = self.all_ids.len();
    }

    /// Prematurely and successfully completes the entire task chain.
    ///
    /// This method marks all remaining (unstarted) stages as "skipped"
    /// and transitions the root task to a successful completion status.
    ///
    /// # Arguments
    ///
    /// * `result_msg` - The final message regarding the successful completion of the root task.
    pub fn finish(&mut self, result_msg: &str) {
        if self.current_stage < self.all_ids.len() {
            self.skip_stages(self.current_stage, "skipped");
        }
        self.inner.set_completed(result_msg);
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    use crate::layout::TaskLayout;
    use crate::layout::anchor::TaskLayoutAnchor;
    use crate::renderer::RenderMode;
    use crate::{TaskBuilder, TaskSequenceBuilder, style::TaskStyle};
    use indicatif::{MultiProgress, ProgressDrawTarget};
    use rstest::rstest;
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

    fn style(mode: &RenderMode) -> Arc<TaskStyle<'static>> {
        Arc::new(match mode {
            RenderMode::Interactive(..) => TaskStyle::modern(),
            RenderMode::Plain(..) => TaskStyle::plain(),
        })
    }

    fn setup_sequence<'a>(stages: &'a [&'a str], mode: RenderMode) -> TaskSequence<'a> {
        let builder = TaskBuilder::new("Test Flow", style(&mode), Arc::new(mode));
        let mut builder = TaskSequenceBuilder::new(builder);
        for stage in stages {
            builder = builder.with_stage(stage);
        }
        builder.start(false)
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_all_ok(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);
        assert_eq!(seq.current_stage, 0);

        seq.ok("A done");
        assert_eq!(seq.current_stage, 1);

        seq.ok("B done");
        assert_eq!(seq.current_stage, 2);

        seq.ok("C done");
        assert_eq!(seq.current_stage, 3);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_all_skipped(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B"], mode);

        seq.skip("Skip A");
        assert_eq!(seq.current_stage, 1);

        seq.skip("Skip B");
        assert_eq!(seq.current_stage, 2);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_fail_at_start_skips_everything(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);
        seq.fail("Critical error on start");

        assert_eq!(seq.current_stage, 3);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_fail_in_middle(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C", "D"], mode);

        seq.ok("A done");
        seq.fail("B failed");

        assert_eq!(seq.current_stage, 4, "Remaining stages must be skipped");
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_fail_on_last_stage(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B"], mode);

        seq.ok("A done");
        seq.fail("B failed");

        assert_eq!(
            seq.current_stage, 2,
            "Should handle failure on the very last stage gracefully"
        );
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_finish_early(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);

        seq.ok("A done");
        seq.finish("Forced finish early");

        assert_eq!(seq.current_stage, 3);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_finish_on_last_stage(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B"], mode);

        seq.ok("A done");
        seq.finish("Finishing instead of OK");

        assert_eq!(seq.current_stage, 2);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_mixed_ok_skip_ok(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);

        seq.ok("A done");
        seq.skip("B skipped");
        seq.ok("C done");

        assert_eq!(seq.current_stage, 3);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_mixed_skip_fail(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);

        seq.skip("A skipped");
        seq.fail("B failed");

        assert_eq!(
            seq.current_stage, 3,
            "Fail after skip should still exhaust the sequence"
        );
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_spam_calls_after_completion(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A"], mode);

        seq.ok("A done");
        assert_eq!(seq.current_stage, 1);

        seq.ok("Ghost call");
        seq.fail("Ghost call");
        seq.skip("Ghost call");
        seq.finish("Ghost call");

        assert_eq!(
            seq.current_stage, 1,
            "State must freeze after sequence is exhausted"
        );
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_empty_sequence_safety(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&[], mode);

        seq.ok("Nothing");
        seq.fail("Nothing");
        seq.skip("Nothing");
        seq.finish("Nothing");

        assert_eq!(seq.current_stage, 0);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_spam_finish_calls(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B"], mode);

        seq.finish("Done early");
        assert_eq!(seq.current_stage, 2);

        seq.finish("Again");
        seq.finish("And again");

        assert_eq!(seq.current_stage, 2);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_single_task_without_stages_manual_finish(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&[], mode);
        seq.finish("done without stages");
        assert_eq!(seq.current_stage, 0);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_sequence_autostart_true(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Auto Flow", style(&mode), Arc::new(mode));
        let mut builder = TaskSequenceBuilder::new(builder);
        builder = builder.with_stage("A");

        let seq = builder.start(false);
        let stage_a_started = {
            let stage = seq.inner.stage_iterator().next().unwrap();
            stage.start_time.lock().unwrap().is_some()
        };
        assert!(stage_a_started);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_sequence_autostart_false(#[case] mode: RenderMode) {
        let builder = TaskBuilder::new("Manual Flow", style(&mode), Arc::new(mode));
        let mut builder = TaskSequenceBuilder::new(builder);
        builder = builder.with_stage("A");

        let seq = builder.build(false);
        let stage_a_started = {
            let stage = seq.inner.stage_iterator().next().unwrap();
            stage.start_time.lock().unwrap().is_some()
        };
        assert!(!stage_a_started);
    }

    #[rstest]
    #[case::interactive(RenderMode::Interactive(layout()))]
    #[case::plain(RenderMode::plain_stdout())]
    fn test_fail_skips_remaining_and_completes(#[case] mode: RenderMode) {
        let mut seq = setup_sequence(&["A", "B", "C"], mode);
        seq.ok("A done");
        seq.fail("B failed");

        assert_eq!(seq.current_stage, 3);
    }
}
