/// A trait for receiving line-by-line text output when in plain rendering mode.
pub trait PlainWriter: Send + Sync {
    /// Writes a single line of text.
    fn write_line(&self, text: &str);
}

/// A default implementation of [`PlainWriter`] that prints directly to standard output.
#[derive(Clone, Default)]
pub struct StdoutWriter;

impl PlainWriter for StdoutWriter {
    fn write_line(&self, text: &str) {
        println!("{}", text);
    }
}
