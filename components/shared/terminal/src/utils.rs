use indicatif::{ProgressBar, ProgressStyle};

pub fn format_indent(level: usize) -> String {
    " ".repeat(level)
}

pub fn create_space() -> ProgressBar {
    let spacer = ProgressBar::new(0);
    spacer.set_style(ProgressStyle::with_template("{msg}").unwrap());
    spacer
}
