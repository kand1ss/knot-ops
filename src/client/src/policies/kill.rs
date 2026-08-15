use tokio::time::Duration;

#[derive(Debug, Clone)]
pub struct KillPolicy {
    pub graceful_timeout: Duration,
    pub force_timeout: Duration,
}

impl Default for KillPolicy {
    fn default() -> Self {
        Self {
            graceful_timeout: Duration::from_millis(200),
            force_timeout: Duration::from_millis(100),
        }
    }
}

impl KillPolicy {
    pub fn new(graceful_timeout: Duration, force_timeout: Duration) -> Self {
        Self { graceful_timeout, force_timeout }
    }
}
