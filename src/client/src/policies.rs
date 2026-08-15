mod kill;
mod timeout;

pub use kill::*;
pub use timeout::*;

#[derive(Debug, Clone, Default)]
pub struct PolicyConfig {
    pub timeout: TimeoutPolicy,
    pub kill: KillPolicy,
}
