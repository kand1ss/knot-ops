mod timeout;

pub use timeout::*;

#[derive(Debug, Clone, Default)]
pub struct PolicyConfig {
    pub(crate) timeout: TimeoutPolicy,
}
