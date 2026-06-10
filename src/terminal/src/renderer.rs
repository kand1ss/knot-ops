mod writer;

use crate::{
    ErrorReport,
    layout::{LayoutNode, TaskLayout},
    style::TaskStyle,
};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::{Duration, Instant};
use std::{borrow::Cow, sync::Arc};
pub use writer::{PlainWriter, StdoutWriter};

/// The active rendering mode for progress bars and output.
#[derive(Clone)]
pub enum RenderMode {
    /// Interactive terminal with live, in-place progress updates.
    Interactive(Arc<TaskLayout>),
    /// Plain output without ANSI codes, suited for non-interactive environments (CI, pipes).
    Plain(Arc<dyn PlainWriter>),
}

impl RenderMode {
    /// Creates a `RenderMode` that writes plain output to stdout.
    pub fn plain_stdout() -> Self {
        Self::Plain(Arc::new(StdoutWriter))
    }
}

pub(crate) enum TaskRenderingStrategy<'a> {
    Interactive(LayoutNode, Arc<TaskLayout>, Vec<LayoutNode>),
    Plain {
        prefix: Cow<'a, str>,
        writer: Arc<dyn PlainWriter>,
    },
}
impl<'a> TaskRenderingStrategy<'a> {
    pub(crate) fn interactive(node: LayoutNode, layout: Arc<TaskLayout>) -> Self {
        Self::Interactive(node, layout, vec![])
    }
    pub(crate) fn plain(prefix: impl Into<Cow<'a, str>>, writer: Arc<dyn PlainWriter>) -> Self {
        Self::Plain {
            prefix: prefix.into(),
            writer,
        }
    }
}

fn write_plain_line(ci_prefix: &str, writer: &dyn PlainWriter, content: &str) {
    let line = if ci_prefix.is_empty() {
        content.to_string()
    } else {
        format!("{} {}", ci_prefix, content)
    };
    writer.write_line(&console::strip_ansi_codes(&line));
}

/// Renders state changes of steps or stages to the terminal.
///
/// `TaskRenderer` abstracts away the formatting of prefixes, spinners, margins,
/// and timings based on the configured rendering style.
pub struct TaskRenderer<'a> {
    /// The display name of the task/step being rendered.
    pub name: Cow<'a, str>,
    pub(crate) strategy: TaskRenderingStrategy<'a>,
    style: Arc<TaskStyle<'a>>,
    indent_spaces: usize,
    baked_indent: String,
    baked_running_indent: String,
    draw_arrow: bool,
}

impl<'a> TaskRenderer<'a> {
    pub(crate) fn new(
        name: impl Into<Cow<'a, str>>,
        strategy: TaskRenderingStrategy<'a>,
        style: Arc<TaskStyle<'a>>,
        indent_spaces: usize,
    ) -> Self {
        let name = name.into();
        let baked_indent = " ".repeat(indent_spaces);
        let draw_arrow = indent_spaces > 2;

        let baked_running_indent = if draw_arrow {
            let arrow_len = style.prefix_active.chars().count();
            " ".repeat(indent_spaces.saturating_sub(arrow_len))
        } else {
            baked_indent.clone()
        };

        Self {
            name,
            strategy,
            style,
            indent_spaces,
            baked_indent,
            baked_running_indent,
            draw_arrow,
        }
    }

    fn format_duration(&self, start: Instant) -> String {
        let elapsed = start.elapsed().as_secs_f32();
        style(format!("[{:.2}s]", elapsed)).dim().to_string()
    }

    /// Renders the task/step title to the output.
    pub fn title(&self) {
        let name = &self.name;
        let prefix = &self.style.prefix_title;
        let indent = &self.baked_indent;

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let template = format!("{indent}{{prefix}} {{msg}}");

                node.set_style(ProgressStyle::with_template(&template).unwrap());
                node.set_prefix(style(prefix.to_string()).cyan().to_string());
                node.set_message(self.name.to_string());
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let content = format!("{indent}{} {}", prefix, name);
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step active or running state, showing a spinner.
    ///
    /// # Arguments
    ///
    /// * `msg` - An optional status message detailing the current sub-operation.
    pub fn run(&self, msg: Option<&str>) {
        let name = &self.name;
        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let prefix_part = if self.draw_arrow {
                    let arrow = &self.style.prefix_active;
                    let styled_arrow = style(arrow.to_string()).dim().to_string();
                    format!("{}{}", self.baked_running_indent, styled_arrow)
                } else {
                    self.baked_running_indent.clone()
                };

                let template = format!("{prefix_part}{{spinner:.cyan}}  {{msg}}");
                node.set_style(
                    ProgressStyle::with_template(&template)
                        .unwrap()
                        .tick_chars(&self.style.run_frame),
                );
                node.enable_steady_tick(Duration::from_millis(80));

                let final_msg = if let Some(msg) = msg {
                    let msg = style(format!("({})", msg)).dim().to_string();
                    format!("{} {}", name, msg)
                } else {
                    self.name.to_string()
                };
                node.set_message(final_msg);
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let status_msg = msg.map(|m| format!(" ({})", m)).unwrap_or_default();
                let start_token = &self.style.run_frame;

                let prefix_part = if self.draw_arrow {
                    format!("{}{}", self.baked_running_indent, self.style.prefix_active)
                } else {
                    self.baked_running_indent.clone()
                };

                let content = format!("{prefix_part}{start_token} {name}{status_msg}");
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step waiting state, indicating it is queued.
    ///
    /// # Arguments
    ///
    /// * `msg` - An optional message explaining what the task is waiting for.
    pub fn wait(&self, msg: Option<&str>) {
        let name = &self.name;
        let indent = &self.baked_indent;
        let prefix = &self.style.prefix_wait;

        let final_msg = if let Some(msg) = msg {
            let msg = format!("({})", msg);
            format!("{} {}", name, msg)
        } else {
            self.name.to_string()
        };

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let template = format!("{indent}{{prefix}} {{msg}}");
                node.set_style(ProgressStyle::with_template(&template).unwrap());
                node.set_prefix(style(prefix.to_string()).dim().to_string());
                node.set_message(style(final_msg).dim().to_string());
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let content = format!("{indent}{prefix} {final_msg}");
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step skipped state.
    ///
    /// # Arguments
    ///
    /// * `msg` - The reason why this step was skipped.
    pub fn skip(&self, msg: &str) {
        let name = &self.name;
        let indent = &self.baked_indent;
        let prefix = &self.style.prefix_skip;
        let final_msg = format!("{} {}", name, style(format!("({})", msg)).dim());

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let template = format!("{indent}{{prefix}} {{msg}}");
                node.set_style(ProgressStyle::with_template(&template).unwrap());
                node.set_prefix(style(prefix.to_string()).dim().to_string());
                node.finish_with_message(final_msg);
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let content = format!("{indent}{prefix} {name} ({msg})");
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step success state along with elapsed execution time.
    ///
    /// # Arguments
    ///
    /// * `msg` - The completion success message.
    /// * `start` - The start timestamp to measure duration.
    pub fn success(&self, msg: &str, start: Instant) {
        let name = &self.name;
        let indent = &self.baked_indent;
        let prefix = &self.style.prefix_success;

        let final_msg = format!(
            "{} {} {}",
            name,
            style(format!("({})", msg)).green(),
            self.format_duration(start)
        );

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let template = format!("{indent}{{prefix}} {{msg}}");
                node.set_style(ProgressStyle::with_template(&template).unwrap());
                node.set_prefix(style(prefix.to_string()).green().to_string());
                node.finish_with_message(final_msg);
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let duration = self.format_duration(start);
                let content = format!("{indent}{prefix} {name} ({msg}) {duration}");
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step failure state along with elapsed execution time.
    ///
    /// # Arguments
    ///
    /// * `start` - The start timestamp to measure duration.
    pub fn fail(&self, start: Instant) {
        let name = &self.name;
        let indent = &self.baked_indent;
        let prefix = &self.style.prefix_fail;

        let final_msg = format!(
            "{} {} {}",
            self.name,
            style("(fail)").red(),
            self.format_duration(start)
        );

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let template = format!("{indent}{{prefix}} {{msg}}");
                node.set_style(ProgressStyle::with_template(&template).unwrap());
                node.set_prefix(style(prefix.to_string()).red().to_string());
                node.finish_with_message(final_msg);
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let duration = self.format_duration(start);
                let content = format!("{indent}{prefix} {name} (fail) {duration}");
                write_plain_line(ci_prefix, writer.as_ref(), &content);
            }
        }
    }

    /// Renders the task/step failure and inserts the detailed [`ErrorReport`] output block.
    ///
    /// # Arguments
    ///
    /// * `error` - The detailed error payload.
    /// * `start` - The start timestamp to measure duration.
    pub fn fail_insert(&mut self, error: impl Into<ErrorReport>, start: Instant) {
        let error = error.into().print(self.indent_spaces);
        self.fail(start);
        match &mut self.strategy {
            TaskRenderingStrategy::Interactive(node, layout, inserted) => {
                let error_bar = layout.insert_after(node, ProgressBar::new(0));
                error_bar.set_style(ProgressStyle::with_template("{msg}").unwrap());
                error_bar.finish_with_message(error);
                inserted.push(error_bar);
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                let clean_error = console::strip_ansi_codes(&error).to_string();
                for line in clean_error.lines() {
                    write_plain_line(ci_prefix, writer.as_ref(), line);
                }
            }
        }
    }

    /// Renders a horizontal divider line.
    pub fn separator(&self) {
        const SEPARATOR_LEN: usize = 100;
        const HEADER_START_IDX: usize = 10;
        const HEADER_SPACE_SIZE: usize = 2;

        let name = &self.name;
        let div_char = &self.style.divider;
        let indent = &self.baked_indent;

        let divider_raw = if !name.is_empty() {
            let header_len = name.chars().count();
            if header_len > SEPARATOR_LEN {
                format!("{indent}{name}")
            } else {
                let start_divider_len = HEADER_START_IDX.saturating_sub(HEADER_SPACE_SIZE);
                let consumed_len = start_divider_len + (2 * HEADER_SPACE_SIZE) + header_len;
                let remaining_divider_len = SEPARATOR_LEN.saturating_sub(consumed_len);

                let start_div = div_char.repeat(start_divider_len);
                let end_div = div_char.repeat(remaining_divider_len);
                let space = " ".repeat(HEADER_SPACE_SIZE);

                format!("{indent}{start_div}{space}{name}{space}{end_div}")
            }
        } else {
            let divider = div_char.repeat(SEPARATOR_LEN);
            format!("{indent}{divider}")
        };

        match &self.strategy {
            TaskRenderingStrategy::Interactive(node, _, _) => {
                let divider = style(divider_raw).dim().to_string();
                node.set_style(ProgressStyle::with_template(&divider).unwrap());
                node.finish();
            }
            TaskRenderingStrategy::Plain {
                prefix: ci_prefix,
                writer,
            } => {
                write_plain_line(ci_prefix, writer.as_ref(), &divider_raw);
            }
        }
    }

    /// Finishes the current progress bar silently without clearing it or changing prefix status.
    pub fn finish_silently(&self) {
        if let TaskRenderingStrategy::Interactive(node, _, inserted) = &self.strategy {
            node.finish();
            for item in inserted {
                item.finish();
            }
        }
    }

    pub(crate) fn clear(&mut self) -> Vec<LayoutNode> {
        let mut cleared_nodes = Vec::new();

        if let TaskRenderingStrategy::Interactive(node, _layout, inserted) = &mut self.strategy {
            node.finish_and_clear();
            cleared_nodes.push(node.clone());

            for ins_node in inserted.drain(..) {
                ins_node.finish_and_clear();
                cleared_nodes.push(ins_node);
            }
        }

        cleared_nodes
    }
}

#[cfg(test)]
mod tests {
    use crate::layout::anchor::TaskLayoutAnchor;

    use super::*;
    use indicatif::{InMemoryTerm, MultiProgress, ProgressBar, ProgressDrawTarget};
    use insta::assert_snapshot;
    use regex::Regex;
    use rstest::rstest;
    use std::sync::Arc;
    use std::time::Instant;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestMode {
        Interactive,
        Plain,
    }

    #[derive(Default)]
    struct TestWriter {
        lines: std::sync::Mutex<Vec<String>>,
    }

    impl PlainWriter for TestWriter {
        fn write_line(&self, text: &str) {
            self.lines.lock().unwrap().push(text.to_string());
        }
    }

    struct TestContext<'a> {
        renderer: TaskRenderer<'a>,
        pb: Option<LayoutNode>,
        term: Option<InMemoryTerm>,
        writer: Option<Arc<TestWriter>>,
        mode: TestMode,
    }

    impl<'a> TestContext<'a> {
        fn new(name: &'a str, mode: TestMode, indent_spaces: usize) -> Self {
            match mode {
                TestMode::Interactive => {
                    let term = InMemoryTerm::new(100, 200);
                    let draw_target = ProgressDrawTarget::term_like(Box::new(term.clone()));
                    let mp = MultiProgress::with_draw_target(draw_target);

                    let anchor = TaskLayoutAnchor::new(None);
                    let layout = TaskLayout::new(mp, anchor);
                    let node = layout.insert(ProgressBar::new(0));

                    let style = Arc::new(TaskStyle::modern());
                    let renderer = TaskRenderer::new(
                        name,
                        TaskRenderingStrategy::Interactive(node.clone(), Arc::new(layout), vec![]),
                        style,
                        indent_spaces,
                    );

                    Self {
                        renderer,
                        pb: Some(node),
                        term: Some(term),
                        writer: None,
                        mode,
                    }
                }
                TestMode::Plain => {
                    let writer = Arc::new(TestWriter::default());
                    let style = Arc::new(TaskStyle::plain());
                    let renderer = TaskRenderer::new(
                        name,
                        TaskRenderingStrategy::Plain {
                            prefix: Cow::Borrowed("[CI]"),
                            writer: writer.clone(),
                        },
                        style,
                        indent_spaces,
                    );

                    Self {
                        renderer,
                        pb: None,
                        term: None,
                        writer: Some(writer),
                        mode,
                    }
                }
            }
        }

        fn tick(&self) {
            if let Some(ref pb) = self.pb {
                pb.tick();
            }
        }

        fn snapshot_name(&self, base: &str) -> String {
            let suffix = match self.mode {
                TestMode::Interactive => "interactive",
                TestMode::Plain => "plain",
            };
            format!("{}_{}", base, suffix)
        }

        fn sanitized_output(&self) -> String {
            let raw = match self.mode {
                TestMode::Interactive => self.term.as_ref().unwrap().contents(),
                TestMode::Plain => self
                    .writer
                    .as_ref()
                    .unwrap()
                    .lines
                    .lock()
                    .unwrap()
                    .join("\n"),
            };
            let re_bracket_time = Regex::new(r"\[\d+\.\d+\s?s\]").unwrap();
            let re_spinner = Regex::new(r"[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]").unwrap();
            let step1 = re_bracket_time.replace_all(&raw, "[TIME]");
            let step2 = re_spinner.replace_all(&step1, "*");
            step2.to_string()
        }
    }

    #[rstest]
    fn test_indent_level_0_no_arrow(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let ctx = TestContext::new("Root Task", mode, 0);

        ctx.renderer.run(None);
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("running_indent_0"),
            ctx.sanitized_output()
        );

        ctx.renderer.success("Done", Instant::now());
        assert_snapshot!(
            ctx.snapshot_name("completed_indent_0"),
            ctx.sanitized_output()
        );
    }

    #[rstest]
    fn test_indent_level_2_no_arrow(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let ctx = TestContext::new("Sub Task", mode, 2);

        ctx.renderer.run(None);
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("running_indent_2"),
            ctx.sanitized_output()
        );
    }

    #[rstest]
    fn test_indent_level_4_with_arrow(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let mut ctx = TestContext::new("Deep Stage", mode, 4);

        ctx.renderer.run(None);
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("running_indent_4_arrow"),
            ctx.sanitized_output()
        );

        ctx.renderer.fail_insert("Error", Instant::now());
        assert_snapshot!(ctx.snapshot_name("failed_indent_4"), ctx.sanitized_output());
    }

    #[rstest]
    fn test_full_lifecycle_transitions(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let ctx = TestContext::new("Build Core", mode, 4);

        ctx.renderer.title();
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("lifecycle_1_title"),
            ctx.sanitized_output()
        );

        ctx.renderer.wait(Some("pending"));
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("lifecycle_2_waiting"),
            ctx.sanitized_output()
        );

        ctx.renderer.run(Some("compiling src/main.rs"));
        ctx.tick();
        assert_snapshot!(
            ctx.snapshot_name("lifecycle_3_running"),
            ctx.sanitized_output()
        );

        ctx.renderer.skip("cached");
        assert_snapshot!(
            ctx.snapshot_name("lifecycle_4_skipped"),
            ctx.sanitized_output()
        );
    }

    #[rstest]
    fn test_optional_messages(#[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode) {
        let ctx = TestContext::new("Task A", mode, 0);

        ctx.renderer.wait(None);
        ctx.tick();
        assert_snapshot!(ctx.snapshot_name("msg_none"), ctx.sanitized_output());

        ctx.renderer.wait(Some("waiting for lock"));
        ctx.tick();
        assert_snapshot!(ctx.snapshot_name("msg_some"), ctx.sanitized_output());
    }

    #[rstest]
    fn test_separator_empty(#[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode) {
        let ctx = TestContext::new("", mode, 0);

        ctx.renderer.separator();
        assert_snapshot!(ctx.snapshot_name("separator_empty"), ctx.sanitized_output());
    }

    #[rstest]
    fn test_separator_short_header(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let ctx = TestContext::new("Phase 2", mode, 4);

        ctx.renderer.separator();
        assert_snapshot!(ctx.snapshot_name("separator_short"), ctx.sanitized_output());
    }

    #[rstest]
    fn test_separator_very_long_header(
        #[values(TestMode::Interactive, TestMode::Plain)] mode: TestMode,
    ) {
        let long_header = "A".repeat(110);
        let ctx = TestContext::new(&long_header, mode, 0);

        ctx.renderer.separator();
        assert_snapshot!(ctx.snapshot_name("separator_long"), ctx.sanitized_output());
    }

    #[test]
    fn test_plain_mode_does_not_panic() {
        let style = Arc::new(TaskStyle::plain());
        let mut renderer = TaskRenderer::new(
            "Test Task",
            TaskRenderingStrategy::plain("[CI]", Arc::new(StdoutWriter)),
            style,
            4,
        );

        renderer.title();
        renderer.wait(None);
        renderer.run(Some("working"));
        renderer.success("done", Instant::now());
        renderer.fail_insert("error", Instant::now());
        renderer.skip("skip");
        renderer.separator();
    }
}
