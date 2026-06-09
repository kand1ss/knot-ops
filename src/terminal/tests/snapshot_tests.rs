use indicatif::{InMemoryTerm, MultiProgress, ProgressDrawTarget, TermLike};
use insta::assert_snapshot;
use knot_terminal::{
    ErrorReport, TaskEngine,
    renderer::{PlainWriter, RenderMode},
    test_utils::layout,
};
use regex::Regex;
use rstest::rstest;
use std::sync::Arc;

#[derive(Clone)]
pub struct InMemoryPlainWriter {
    term: InMemoryTerm,
}

impl PlainWriter for InMemoryPlainWriter {
    fn write_line(&self, text: &str) {
        let _ = indicatif::TermLike::write_line(&self.term, text);
    }
}

#[derive(Debug, Clone, Copy)]
enum TestMode {
    Interactive,
    Plain,
}

fn snapshot_name(base: &str, mode: TestMode) -> String {
    let suffix = match mode {
        TestMode::Interactive => "interactive",
        TestMode::Plain => "plain",
    };
    format!("{}_{}", base, suffix)
}

pub fn sanitized_output(term: &InMemoryTerm) -> String {
    let raw = term.contents();
    let re_bracket_time = Regex::new(r"\[\s*\d+\s*\.\s*\d+\s*s\s*\]").unwrap();
    let re_spinner = Regex::new(r"[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]").unwrap();
    let step1 = re_bracket_time.replace_all(&raw, "[TIME]");
    let step2 = re_spinner.replace_all(&step1, "*");
    step2.to_string()
}

fn create_engine(mode: TestMode) -> (TaskEngine<'static>, InMemoryTerm) {
    let term = InMemoryTerm::new(200, 150);
    let multi =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(term.clone())));

    let plain_writer = InMemoryPlainWriter { term: term.clone() };

    let engine = TaskEngine::with_multi(multi.clone());
    let engine = match mode {
        TestMode::Interactive => engine.with_render_mode(RenderMode::Interactive(layout(multi))),
        TestMode::Plain => engine.with_render_mode(RenderMode::Plain(Arc::new(plain_writer))),
    };

    (engine, term)
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn task_init(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Basic Task")
        .with_stage("s1", "Step One", false)
        .with_stage("s2", "Step Two", false)
        .start(false);

    assert_snapshot!(snapshot_name("task_init", mode), sanitized_output(&term))
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_stages_one_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("One Stage is Ok")
        .with_stage("s1", "Step One", false)
        .with_stage("s2", "Step Two", false)
        .start(false);

    task.ok_by_id("s2", "Ok");
    assert_snapshot!(
        snapshot_name("two_stages_one_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn single_stage_task_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Basic Task")
        .with_stage("s1", "Step One", false)
        .start(false);

    task.run_by_id("s1", None);
    task.ok_by_id("s1", "done");

    assert_snapshot!(
        snapshot_name("single_stage_task_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn task_with_auto_indent_has_trailing_space(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Indented Task")
        .with_stage("s1", "Step One", false)
        .start(true);

    task.run_by_id("s1", None);
    let _test_task = engine.task("Space?").start(false);

    assert_snapshot!(
        snapshot_name("task_with_auto_indent", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn simple_task_ok_success(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine.task("Simple Task").start(false);

    task.ok("success");
    assert_snapshot!(
        snapshot_name("simple_task_ok_sucess", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn single_stage_task_skipped(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Skipped Task")
        .with_stage("s1", "Step One", false)
        .start(false);

    task.run_by_id("s1", None);
    task.skip_by_id("s1", "not needed");

    assert_snapshot!(
        snapshot_name("single_stage_task_skipped", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn single_stage_task_failed(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Failed Task")
        .with_stage("s1", "Step One", false)
        .start(false);

    task.run_by_id("s1", None);
    task.fail_by_id("s1", "something went wrong");

    assert_snapshot!(
        snapshot_name("single_stage_task_failed", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_stages_one_failed(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Failed Task")
        .with_stage("s1", "Step One", false)
        .with_stage("s2", "Step Two", false)
        .start(false);

    task.run_by_id("s1", Some("Running"));
    task.fail_by_id("s1", "Failed");

    assert_snapshot!(
        snapshot_name("two_stages_one_failed", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_stages_one_failed_full_error(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Failed Task")
        .with_stage("s1", "Step One", false)
        .with_stage("s2", "Step Two", false)
        .start(false);

    task.run_by_id("s1", Some("Running"));
    task.fail_by_id(
        "s1",
        ErrorReport::new("Fail")
            .with_solution("No Solution")
            .with_context("No Context"),
    );

    assert_snapshot!(
        snapshot_name("two_stages_one_failed_full_error", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn engine_space_inserts_blank_line(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    let mut t = engine
        .task("Task Before Space")
        .with_stage("s1", "Step", false)
        .start(false);
    t.ok_by_id("s1", "done");

    engine.space();

    let mut t2 = engine
        .task("Task After Space")
        .with_stage("s2", "Step", false)
        .start(false);
    t2.ok_by_id("s2", "done");

    assert_snapshot!(
        snapshot_name("engine_space_insers_blank_line", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn single_stage_fail_marks_task_failed(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Failing Task")
        .with_stage("s1", "Compile", false)
        .start(false);

    task.run_by_id("s1", None);
    task.fail_by_id("s1", "compile error");

    assert_snapshot!(
        snapshot_name("single_stage_fail_marks_task_failed", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_stage_task_second_fails(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Two Stage Fail")
        .with_stage("s1", "Prepare", false)
        .with_stage("s2", "Execute", false)
        .start(false);

    task.run_by_id("s1", None);
    task.ok_by_id("s1", "prepared");

    task.run_by_id("s2", None);
    task.fail_by_id("s2", "execution error");

    assert_snapshot!(
        snapshot_name("two_stage_task_second_fails", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_fail_first_stage_skips_rest(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Error Sequence")
        .with_stage("Stage A")
        .with_stage("Stage B")
        .with_stage("Stage C")
        .start(false);

    seq.fail("critical error");

    assert_snapshot!(
        snapshot_name("sequence_fail_first_stage_skips_rest", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_fail_middle_remaining_skipped(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Middle Error Sequence")
        .with_stage("Stage A")
        .with_stage("Stage B")
        .with_stage("Stage C")
        .with_stage("Stage D")
        .start(false);

    seq.ok("A done");
    seq.ok("B done");
    seq.fail("C failed");

    assert_snapshot!(
        snapshot_name("sequence_fail_middle_remaining_skipped", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_fail_last_stage(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Last Stage Error")
        .with_stage("Stage A")
        .with_stage("Stage B")
        .start(false);

    seq.ok("A done");
    seq.fail("B failed");

    assert_snapshot!(
        snapshot_name("sequence_fail_last_stage", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_ghost_calls_after_failure_ignored(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Ghost Calls After Fail")
        .with_stage("Stage A")
        .start(false);

    seq.fail("fail");

    seq.ok("ghost");
    seq.skip("ghost");
    seq.fail("ghost");
    seq.finish("ghost");

    assert_snapshot!(
        snapshot_name("sequence_ghost_calls_after_failure_ignored", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn failed_task_does_not_affect_subsequent_task(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    let mut t1 = engine
        .task("Task That Fails")
        .with_stage("s1", "Step", false)
        .start(false);
    t1.run_by_id("s1", None);
    t1.fail_by_id("s1", "failed");

    let mut t2 = engine
        .task("Task That Succeeds")
        .with_stage("s2", "Step", false)
        .start(false);
    t2.run_by_id("s2", None);
    t2.ok_by_id("s2", "done");

    let output = sanitized_output(&term);
    assert!(output.contains("Task That Fails"));
    assert!(output.contains("Task That Succeeds"));
    assert_snapshot!(
        snapshot_name("failed_task_does_not_affect_subsequent_task", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn three_stage_task_all_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Multi Stage")
        .with_stage("s1", "Download", false)
        .with_stage("s2", "Build", false)
        .with_stage("s3", "Deploy", false)
        .start(false);

    task.run_by_id("s1", None);
    task.ok_by_id("s1", "downloaded");

    task.run_by_id("s2", None);
    task.ok_by_id("s2", "built");

    task.run_by_id("s3", None);
    task.ok_by_id("s3", "deployed");

    assert_snapshot!(
        snapshot_name("three_stage_task_all_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn three_stage_task_autorun_first_skip_last(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Mixed Stages")
        .with_stage("s1", "Fetch", true)
        .with_stage("s2", "Process", false)
        .with_stage("s3", "Upload", false)
        .start(false);

    task.ok_by_id("s1", "fetched");
    task.run_by_id("s2", None);
    task.ok_by_id("s2", "processed");

    task.skip_by_id("s3", "already up to date");

    assert_snapshot!(
        snapshot_name("three_stage_task_autorun_first_skip_last", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn three_stage_task_middle_fails(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Failing Multi Stage")
        .with_stage("s1", "Download", false)
        .with_stage("s2", "Build", false)
        .with_stage("s3", "Deploy", false)
        .start(false);

    task.run_by_id("s1", None);
    task.ok_by_id("s1", "done");

    task.run_by_id("s2", None);
    task.fail_by_id("s2", "compilation error");

    assert_snapshot!(
        snapshot_name("three_stage_task_middle_fails", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn bulk_ok_completes_all_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine
        .task("Bulk OK Task")
        .with_stage("s1", "Stage A", false)
        .with_stage("s2", "Stage B", false)
        .start(false);

    task.ok("all done");

    assert_snapshot!(
        snapshot_name("bulk_ok_completes_all_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn bulk_skip_all_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine
        .task("Bulk Skip Task")
        .with_stage("s1", "Stage A", false)
        .with_stage("s2", "Stage B", false)
        .start(false);

    task.skip("cached");

    assert_snapshot!(
        snapshot_name("bulk_skip_all_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn bulk_fail_when_no_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine.task("Bulk Fail Task").start(false);

    task.fail(
        ErrorReport::new("error")
            .with_context("context")
            .with_solution("solution"),
    );

    assert_snapshot!(
        snapshot_name("bulk_fail_when_no_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_all_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Deployment")
        .with_stage("Pull images")
        .with_stage("Start containers")
        .with_stage("Healthcheck")
        .start(false);

    seq.ok("pulled");
    seq.ok("started");
    seq.ok("healthy");

    assert_snapshot!(
        snapshot_name("sequence_all_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_first_fails_rest_skipped(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("CI Pipeline")
        .with_stage("Compile")
        .with_stage("Test")
        .with_stage("Publish")
        .start(false);

    seq.fail("syntax error");

    assert_snapshot!(
        snapshot_name("sequence_first_fails_rest_skipped", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_middle_stage_fails(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("CI Pipeline")
        .with_stage("Compile")
        .with_stage("Test")
        .with_stage("Publish")
        .start(false);

    seq.ok("compiled");
    seq.fail("test suite failed");

    assert_snapshot!(
        snapshot_name("sequence_middle_stage_fails", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_mixed_ok_and_skip(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Incremental Build")
        .with_stage("Fetch dependencies")
        .with_stage("Compile")
        .with_stage("Bundle")
        .start(false);

    seq.ok("fetched");
    seq.skip("already compiled");
    seq.ok("bundled");

    assert_snapshot!(
        snapshot_name("sequence_mixed_ok_and_skip", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_early_finish(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Conditional Deploy")
        .with_stage("Check cache")
        .with_stage("Build")
        .with_stage("Push")
        .start(false);

    seq.ok("cache hit");
    seq.finish("nothing to do");

    assert_snapshot!(
        snapshot_name("sequence_early_finish", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_sequences_separated_by_space_no_duplication(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    let mut seq1 = engine
        .sequence("First Job")
        .with_stage("Clone")
        .with_stage("Build")
        .start(false);
    seq1.ok("cloned");
    seq1.ok("built");

    engine.space();

    let mut seq2 = engine
        .sequence("Second Job")
        .with_stage("Deploy")
        .start(false);
    seq2.ok("deployed");

    let output = sanitized_output(&term);

    assert!(output.contains("First Job"));
    assert!(output.contains("Second Job"));
    assert_snapshot!(
        snapshot_name("two_sequences_separated_by_space_no_duplication", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn regression_sequence_space_task_no_anchor_bug(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    let mut seq = engine
        .sequence("Seq Task")
        .with_stage("Stage 1")
        .build(false);
    seq.ok("Stage 1 done");
    seq.finish("done");

    engine.space();

    let mut t = engine
        .task("After Space Task")
        .with_stage("s1", "Final Step", false)
        .start(false);
    t.ok_by_id("s1", "ok");

    let output = sanitized_output(&term);

    assert!(output.contains("After Space Task"));
    assert!(output.contains("Seq Task"));
    assert_snapshot!(
        snapshot_name("regression_sequence_space_task_no_anchor_bug", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn calls_after_sequence_exhausted_are_ignored(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("One Stage Job")
        .with_stage("Only Stage")
        .start(false);

    seq.ok("done");
    seq.ok("ghost");
    seq.fail("ghost");
    seq.skip("ghost");
    seq.finish("ghost");

    assert_snapshot!(
        snapshot_name("calls_after_sequence_exhausted_are_ignored", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_at_the_start(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Group")
        .with_group(Some("Header"))
        .with_stage("s1", "Stage 1", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("group_at_the_start", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_in_the_middle(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Group In Middle")
        .with_stage("s1", "Initial Stage", false)
        .with_group(Some("Middle Group"))
        .with_stage("s2", "Grouped Stage", false)
        .with_stage("s3", "Another Grouped Stage", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("group_in_the_middle", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_at_the_end(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Group At End")
        .with_stage("s1", "Initial Stage", false)
        .with_stage("s2", "Final Stage 1", false)
        .with_stage("s3", "Final Stage 2", false)
        .with_group(Some("Final Group"))
        .start(false);

    assert_snapshot!(
        snapshot_name("group_in_the_end", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn only_group_no_root_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Only Group")
        .with_group(Some("Only Header"))
        .with_stage("s1", "Stage 1", false)
        .with_stage("s2", "Stage 2", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("only_group_no_root_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn consecutive_groups(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Consecutive Groups")
        .with_group(Some("Group A"))
        .with_stage("a1", "Stage A1", false)
        .with_group(Some("Group B"))
        .with_stage("b1", "Stage B1", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("consecutive_groups", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn interleaved_groups_and_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Interleaved")
        .with_group(Some("Group 1"))
        .with_stage("g1_1", "Stage G1", false)
        .with_group(None)
        .with_stage("root_1", "Root Stage", false)
        .with_group(Some("Group 2"))
        .with_stage("g2_1", "Stage G2", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("interleaved_groups_and_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_with_extremely_long_header(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let long_header = "X".repeat(999);

    let _task = engine
        .task("Long Header Task")
        .with_group(Some(&long_header))
        .with_stage("s1", "Stage 1", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("group_with_extremely_long_header", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn empty_group_with_no_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Empty Group Task")
        .with_group(Some("Empty Header"))
        .with_group(Some("Next Group"))
        .with_stage("s1", "Stage 1", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("empty_group_with_no_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_with_no_header_text(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _task = engine
        .task("Nameless Group")
        .with_group(Some(""))
        .with_stage("s1", "Stage 1", false)
        .start(false);

    assert_snapshot!(
        snapshot_name("empty_group_with_no_header_text", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn group_all_stages_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Group OK")
        .with_group(Some("Building"))
        .with_stage("compile", "Compile source", false)
        .with_stage("link", "Link binary", false)
        .start(false);

    task.ok_by_id("compile", "ok");
    task.ok_by_id("link", "ok");

    assert_snapshot!(
        snapshot_name("group_all_stages_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn fail_inside_group(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Group Fail")
        .with_group(Some("Testing"))
        .with_stage("t1", "Unit tests", false)
        .with_stage("t2", "Integration tests", false)
        .with_stage("t3", "E2E tests", false)
        .start(false);

    task.ok_by_id("t1", "ok");
    task.fail_by_id("t2", "Integration failed");

    assert_snapshot!(
        snapshot_name("fail_inside_group", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn fail_before_group_aborts_group_stages(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Fail Before Group")
        .with_stage("init", "Init workspace", false)
        .with_group(Some("Deploy"))
        .with_stage("d1", "Upload files", false)
        .with_stage("d2", "Restart service", false)
        .start(false);

    task.fail_by_id("init", "Disk full");

    assert_snapshot!(
        snapshot_name("fail_before_group_aborts_group_stages", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn skip_stage_inside_group(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Skip Inside Group")
        .with_group(Some("Setup"))
        .with_stage("s1", "Download cache", false)
        .with_stage("s2", "Extract cache", false)
        .start(false);

    task.skip_by_id("s1", "Cache found locally");
    task.ok_by_id("s2", "ok");

    assert_snapshot!(
        snapshot_name("skip_stage_inside_group", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn single_group_failed_task(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine
        .task("Single Group Failed Task")
        .with_group(Some("Setup"))
        .start(false);

    task.fail("error");
    assert_snapshot!(
        snapshot_name("single_group_failed_task", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_with_groups_success(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Grouped Sequence")
        .with_group(Some("Prep"))
        .with_stage("Clean")
        .with_stage("Fetch")
        .with_group(Some("Build"))
        .with_stage("Compile")
        .start(false);

    seq.ok("Cleaned");
    seq.ok("Fetched");
    seq.ok("Compiled");

    assert_snapshot!(
        snapshot_name("sequence_with_groups_success", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn sequence_fail_inside_group(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Grouped Sequence Fail")
        .with_group(Some("Prep"))
        .with_stage("Clean")
        .with_stage("Fetch")
        .start(false);

    seq.ok("Cleaned");
    seq.fail("Network error");

    assert_snapshot!(
        snapshot_name("sequence_fail_inside_group", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_simple_task_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine.task("Simple Task").start(false);

    task.ok("Finished cleanly");

    term.write_line(">>> [STDOUT] Plain text after simple task OK")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_simple_task_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_simple_task_fail(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let task = engine.task("Simple Failing Task").start(false);

    task.fail("Critical error occurred");
    term.write_line(">>> [STDOUT] Plain text after simple task FAIL")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_simple_task_fail", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_staged_task_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Staged Task")
        .with_stage("s1", "Download", false)
        .with_stage("s2", "Extract", false)
        .start(false);

    task.ok_by_id("s1", "ok");
    task.ok_by_id("s2", "ok");

    term.write_line(">>> [STDOUT] Plain text after staged task OK")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_staged_task_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_staged_task_fail(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Staged Task Failing")
        .with_stage("s1", "Download", false)
        .with_stage("s2", "Extract", false)
        .with_stage("s3", "Build", false)
        .start(false);

    task.ok_by_id("s1", "ok");
    task.fail_by_id("s2", "Archive corrupted");

    term.write_line(">>> [STDOUT] Plain text after staged task FAIL")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_staged_task_fail", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_grouped_task_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Grouped Task")
        .with_group(Some("Initialization"))
        .with_stage("init", "Init DB", false)
        .with_group(Some("Migrations"))
        .with_stage("m1", "Apply V1", false)
        .with_stage("m2", "Apply V2", false)
        .start(false);

    task.ok_by_id("init", "ok");
    task.ok_by_id("m1", "ok");
    task.ok_by_id("m2", "ok");

    term.write_line(">>> [STDOUT] Plain text after grouped task OK")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_grouped_task_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_grouped_task_fail_in_middle(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut task = engine
        .task("Grouped Task Failing")
        .with_group(Some("Network"))
        .with_stage("ping", "Ping server", false)
        .with_group(Some("Auth"))
        .with_stage("login", "Login user", false)
        .with_stage("token", "Fetch token", false)
        .start(false);

    task.ok_by_id("ping", "ok");
    task.fail_by_id("login", "Invalid credentials");

    term.write_line(">>> [STDOUT] Plain text after grouped task FAIL")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_grouped_task_fail_in_middle", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_sequence_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Deployment Pipeline")
        .with_stage("Build")
        .with_stage("Test")
        .start(false);

    seq.ok("Built successfully");
    seq.ok("Tests passed");

    term.write_line(">>> [STDOUT] Plain text after sequence OK")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_sequence_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_sequence_fail(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut seq = engine
        .sequence("Deployment Pipeline Failing")
        .with_stage("Build")
        .with_stage("Test")
        .with_stage("Deploy")
        .start(false);

    seq.ok("Built successfully");
    seq.fail("Tests failed: 5 errors");

    term.write_line(">>> [STDOUT] Plain text after sequence FAIL")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_sequence_fail", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_multiple_independent_tasks(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    let task1 = engine.task("Task 1").start(false);
    task1.ok("Done 1");

    let task2 = engine.task("Task 2").start(false);
    task2.fail("Failed 2");

    term.write_line(">>> [STDOUT] Plain text after multiple tasks")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_multiple_independent_tasks", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_engine_is_dropped(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);

    {
        let task = engine.task("Scoped Task").start(false);
        task.ok("Done");
    }

    drop(engine);

    term.write_line(">>> [STDOUT] Plain text after ENGINE DROP")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_engine_is_dropped", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_happy_path_with_run(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut step = engine.step("Compile Core", false);

    step.run(Some("linking objects"));
    step.ok("compiled in 0.5s");

    assert_snapshot!(
        snapshot_name("step_happy_path_with_run", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_auto_run(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let _step = engine.step("Compile Core", true);
    assert_snapshot!(
        snapshot_name("step_auto_run", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_run(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut step = engine.step("Compile Core", false);

    step.run(Some("linking objects"));
    assert_snapshot!(snapshot_name("step_run", mode), sanitized_output(&term))
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_immediate_ok_without_run(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Check Cache", false);

    step.ok("cache hit");
    assert_snapshot!(
        snapshot_name("step_immediate_ok_without_run", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_multiple_runs_override_status(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let mut step = engine.step("Download", false);

    step.run(Some("10%"));
    step.run(Some("50%"));
    step.run(Some("99%"));
    step.ok("100% done");

    assert_snapshot!(
        snapshot_name("step_multiple_runs_override_status", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_skip_behavior(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Optional Check", true);

    step.skip("not required for this build");
    assert_snapshot!(
        snapshot_name("step_skip_behavior", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_fail_inserts_error_node(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Database Migration", true);

    let error = ErrorReport::new("Duplicate key")
        .with_context("Table 'users'")
        .with_solution("Drop table and retry");

    step.fail(error);
    assert_snapshot!(
        snapshot_name("step_fail_inserts_error_node", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn step_fail_without_run(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Instant Fail", false);

    step.fail(ErrorReport::new("Config not found"));
    assert_snapshot!(
        snapshot_name("step_fail_without_run", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_step_ok(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Simple Step", true);

    step.ok("Done");
    term.write_line(">>> [STDOUT] Should be below the step")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_step_ok", mode),
        sanitized_output(&term)
    )
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn stdout_after_step_fail(#[case] mode: TestMode) {
    let (engine, term) = create_engine(mode);
    let step = engine.step("Failing Step", true);

    step.fail(ErrorReport::new("Fatal"));
    term.write_line(">>> [STDOUT] Should be below the error report")
        .unwrap();
    assert_snapshot!(
        snapshot_name("stdout_after_step_fail", mode),
        sanitized_output(&term)
    )
}

fn setup_two_engines(mode: TestMode) -> (TaskEngine<'static>, TaskEngine<'static>, InMemoryTerm) {
    let term = InMemoryTerm::new(200, 150);
    let multi =
        MultiProgress::with_draw_target(ProgressDrawTarget::term_like(Box::new(term.clone())));
    let plain_writer: Arc<dyn PlainWriter> = Arc::new(InMemoryPlainWriter { term: term.clone() });

    let engine1 = TaskEngine::with_multi(multi.clone());
    let engine1 = match mode {
        TestMode::Interactive => {
            engine1.with_render_mode(RenderMode::Interactive(layout(multi.clone())))
        }
        TestMode::Plain => engine1.with_render_mode(RenderMode::Plain(Arc::clone(&plain_writer))),
    };

    let engine2 = TaskEngine::with_multi(multi.clone());
    let engine2 = match mode {
        TestMode::Interactive => engine2.with_render_mode(RenderMode::Interactive(layout(multi))),
        TestMode::Plain => engine2.with_render_mode(RenderMode::Plain(plain_writer)),
    };

    (engine1, engine2, term)
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_interleaved_stages(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut t1 = e1
        .task("Backend Build")
        .with_stage("b1", "Compile", false)
        .start(false);
    let mut t2 = e2
        .task("Frontend Build")
        .with_stage("f1", "Bundle", false)
        .start(false);

    t1.run_by_id("b1", Some("running rustc"));
    t2.run_by_id("f1", Some("running webpack"));

    t1.ok_by_id("b1", "compiled");
    t2.fail_by_id("f1", "bundle size exceeded");

    t1.ok("Backend done");
    t2.fail("Frontend failed");

    assert_snapshot!(
        snapshot_name("interleaved_stages", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_sequence_vs_standard(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut standard_task = e1
        .task("Database Init")
        .with_stage("db1", "Migrate", false)
        .with_stage("db2", "Seed", false)
        .start(false);

    let mut sequence = e2
        .sequence("Deploy Sequence")
        .with_stage("Upload")
        .with_stage("Restart")
        .start(false);

    standard_task.run_by_id("db1", None);
    sequence.ok("Uploaded successfully");

    standard_task.skip_by_id("db2", "Already seeded");
    standard_task.ok_by_id("db1", "Done");

    sequence.fail("Service crashed");

    assert_snapshot!(
        snapshot_name("sequence_vs_standard", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_concurrent_errors(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut t1 = e1
        .task("Network Sync")
        .with_stage("net", "Connect", false)
        .start(false);
    let mut t2 = e2
        .task("Disk Write")
        .with_stage("io", "Save", false)
        .start(false);

    t1.run_by_id("net", None);
    t2.run_by_id("io", None);

    t1.fail_by_id(
        "net",
        crate::ErrorReport::new("Connection refused").with_context("192.168.1.1"),
    );
    t2.fail_by_id(
        "io",
        crate::ErrorReport::new("Access Denied").with_solution("Run as sudo"),
    );

    assert_snapshot!(
        snapshot_name("concurrent_errors", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_groups_and_separators(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut t1 = e1
        .task("App 1")
        .with_group(Some("Dependencies"))
        .with_stage("d1", "Pulling", false)
        .start(false);

    let mut t2 = e2
        .task("App 2")
        .with_group(Some("Build Steps"))
        .with_stage("b1", "Compiling", false)
        .start(false);

    t1.ok_by_id("d1", "Pulled");
    t2.ok_by_id("b1", "Compiled");

    t1.ok("App 1 Ready");
    t2.ok("App 2 Ready");

    assert_snapshot!(
        snapshot_name("groups_and_separators", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_mixed_auto_run_and_indent(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut t1 = e1
        .task("Fast Task")
        .with_stage("fast1", "Zoom", true)
        .with_stage("fast2", "Swoosh", true)
        .start(true);

    let mut t2 = e2
        .task("Slow Task")
        .with_stage("slow1", "Crawl", false)
        .start(false);

    t1.ok_by_id("fast1", "Done");
    t2.run_by_id("slow1", Some("Started manually"));
    t1.skip_by_id("fast2", "Skipped");
    t2.ok_by_id("slow1", "Finished eventually");

    assert_snapshot!(
        snapshot_name("mixed_auto_run_indent", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_empty_tasks_quick_finish(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let empty_t1 = e1.task("Ghost Protocol").start(false);

    let mut heavy_t2 = e2
        .task("Heavy Lifting")
        .with_stage("s1", "Loading", false)
        .start(false);

    empty_t1.ok("Nothing to do");

    heavy_t2.run_by_id("s1", None);
    heavy_t2.fail_by_id("s1", "Crash");
    heavy_t2.fail("Aborted");

    assert_snapshot!(
        snapshot_name("empty_tasks_quick_finish", mode),
        sanitized_output(&term)
    );
}

#[rstest]
#[case::interactive(TestMode::Interactive)]
#[case::plain(TestMode::Plain)]
fn two_engines_abort_signal(#[case] mode: TestMode) {
    let (e1, e2, term) = setup_two_engines(mode);

    let mut t1 = e1
        .task("Worker 1")
        .with_stage("1", "Job", true)
        .start(false);
    let mut t2 = e2
        .task("Worker 2")
        .with_stage("2", "Job", true)
        .start(false);

    t1.fail_by_id("1", "SIGINT");
    t2.fail_by_id("2", "SIGINT");

    t1.ok("Ghost call");
    t2.ok("Ghost call");

    assert_snapshot!(snapshot_name("abort_signal", mode), sanitized_output(&term));
}
