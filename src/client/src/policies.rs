mod timeout;

pub use timeout::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct PolicyConfig {
    pub(crate) timeout: TimeoutPolicy,
}
