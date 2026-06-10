use std::borrow::Cow;

/// Defines prefix characters, spinners, and dividers for styling task rendering.
#[derive(Clone)]
pub struct TaskStyle<'a> {
    /// Prefix icon for rendering titles.
    pub prefix_title: Cow<'a, str>,
    /// Prefix icon indicating a task or sub-stage is active.
    pub prefix_active: Cow<'a, str>,
    /// Prefix icon indicating success.
    pub prefix_success: Cow<'a, str>,
    /// Prefix icon indicating failure.
    pub prefix_fail: Cow<'a, str>,
    /// Prefix icon indicating skipped state.
    pub prefix_skip: Cow<'a, str>,
    /// Prefix icon indicating waiting state.
    pub prefix_wait: Cow<'a, str>,
    /// Characters used for running state spinner frames.
    pub run_frame: Cow<'a, str>,
    /// Character used to draw horizontal dividers.
    pub divider: Cow<'a, str>,
}

impl<'a> TaskStyle<'a> {
    /// Creates a modern unicode style preset featuring clean icons and a braille spinner.
    pub fn modern() -> Self {
        Self {
            prefix_title: "▶".into(),
            prefix_success: "✔ ".into(),
            prefix_active: "> ".into(),
            prefix_fail: "✖ ".into(),
            prefix_skip: "↷ ".into(),
            prefix_wait: "· ".into(),
            run_frame: "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".into(),
            divider: "─".into(),
        }
    }

    /// Creates a plain ASCII style preset optimized for text/CI environments.
    pub fn plain() -> Self {
        Self {
            prefix_title: "[TASK]".into(),
            prefix_success: "[SUCCESS]".into(),
            prefix_active: "> ".into(),
            prefix_fail: "[FAIL]".into(),
            prefix_skip: "[SKIP]".into(),
            prefix_wait: "[WAIT]".into(),
            run_frame: "[START]".into(),
            divider: "─".into(),
        }
    }
}
