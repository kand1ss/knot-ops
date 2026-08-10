use tokio::time::Duration;

#[derive(Debug, Clone)]
pub struct TimeoutPolicy {
    pub fast_commands: Duration,
    pub long_streams: Option<Duration>,
}
impl Default for TimeoutPolicy {
    fn default() -> Self {
        Self {
            fast_commands: Duration::from_secs(3),
            long_streams: None,
        }
    }
}
