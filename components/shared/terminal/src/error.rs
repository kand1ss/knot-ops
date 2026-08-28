use crate::utils::format_indent;
use console::style;
use std::fmt::Write;

/// A context-rich diagnostic report for failed tasks or steps.
///
/// `ErrorReport` allows you to format error outputs with clear messages,
/// optional contextual details (e.g. raw logs or commands), and actionable solutions.
/// It renders beautifully to the terminal with side-borders and indentation.
#[derive(Clone, Debug)]
pub struct ErrorReport {
    /// The primary error message summarizing the problem.
    pub message: String,
    /// Detailed contextual information (e.g. underlying file paths, stderr output).
    pub context: Option<String>,
    /// Suggested actions or solutions to fix the issue.
    pub solution: Option<String>,
}

impl ErrorReport {
    /// Creates a new `ErrorReport` with a main message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            context: None,
            solution: None,
        }
    }

    /// Appends context information to the error report.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Appends a suggested solution or action step to the error report.
    pub fn with_solution(mut self, solution: impl Into<String>) -> Self {
        self.solution = Some(solution.into());
        self
    }

    /// Formats and returns the error report as a styled multi-line string.
    ///
    /// # Arguments
    ///
    /// * `indent_space` - The number of spaces to indent the output block.
    pub fn print(&self, indent_space: usize) -> String {
        let mut out = String::new();

        let term_width = textwrap::termwidth().min(100);
        let base_indent = format_indent(indent_space);

        let raw_border = format!("{}│ ", base_indent);
        let border = style(&raw_border).red().to_string();

        let indent_spaces = "          ";
        let content_width =
            term_width.saturating_sub(raw_border.chars().count() + indent_spaces.len());

        let _ = writeln!(out, "{}", border);

        let issue_lines = textwrap::wrap(&self.message, content_width);
        for (i, line) in issue_lines.iter().enumerate() {
            if i == 0 {
                let _ = writeln!(
                    out,
                    "{}{}{}",
                    border,
                    style("issue:    ").red().bold(),
                    style(line).bold()
                );
            } else {
                let _ = writeln!(out, "{}{}{}", border, indent_spaces, style(line).bold());
            }
        }

        if let Some(ctx) = &self.context {
            let _ = writeln!(out, "{}", border);

            let clean_ctx = ctx.replace("\\\\?\\", "");
            let ctx_lines = textwrap::wrap(&clean_ctx, content_width);

            for (i, line) in ctx_lines.iter().enumerate() {
                if i == 0 {
                    let _ = writeln!(
                        out,
                        "{}{}{}",
                        border,
                        style("context:  ").yellow(),
                        style(line).dim()
                    );
                } else {
                    let _ = writeln!(out, "{}{}{}", border, indent_spaces, style(line).dim());
                }
            }
        }

        if let Some(sol) = &self.solution {
            let _ = writeln!(out, "{}", border);

            let sol_lines = textwrap::wrap(sol, content_width);
            for (i, line) in sol_lines.iter().enumerate() {
                if i == 0 {
                    let _ = writeln!(
                        out,
                        "{}{}{}",
                        border,
                        style("solution: ").green().bold(),
                        line
                    );
                } else {
                    let _ = writeln!(out, "{}{}{}", border, indent_spaces, line);
                }
            }
        }

        out.trim_end().to_string()
    }
}

impl From<&str> for ErrorReport {
    fn from(s: &str) -> Self {
        ErrorReport::new(s)
    }
}

impl From<String> for ErrorReport {
    fn from(s: String) -> Self {
        ErrorReport::new(s)
    }
}
