/// Configuration options for establishing an incoming IPC connection listener.
///
/// This struct allows fine-tuning the behavior of the `IpcIncoming` server,
/// primarily controlling how many pending connections can be queued before
/// backpressure is applied to the listener.
#[derive(Debug, Clone)]
pub struct IncomingOptions {
    /// The number of pending connections to buffer in the internal MPSC channel.
    ///
    /// This defines the capacity of the channel that holds accepted but not
    /// yet processed connections. A larger buffer helps absorb bursts of
    /// incoming connections but increases memory usage.
    pub buffer_size: usize,
}

impl Default for IncomingOptions {
    /// Creates a default configuration with a buffer size of 64.
    fn default() -> Self {
        Self { buffer_size: 64 }
    }
}
