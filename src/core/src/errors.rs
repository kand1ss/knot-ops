use thiserror::Error;
mod config;
mod daemon;
mod services;
mod transport;

pub use {
    config::ConfigError, daemon::DaemonError, services::ServiceError, transport::TransportError,
};

#[derive(Debug, Error)]
pub enum KnotError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Service(#[from] ServiceError),
}
