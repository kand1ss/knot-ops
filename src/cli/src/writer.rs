use indicatif::MultiProgress;
use std::io::Write;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
pub struct MultiProgressWriter {
    multi: MultiProgress,
}

impl MultiProgressWriter {
    pub fn new(multi: MultiProgress) -> Self {
        Self { multi }
    }
}

impl Write for MultiProgressWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.multi.suspend(|| std::io::stderr().write_all(buf))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

impl<'a> MakeWriter<'a> for MultiProgressWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
