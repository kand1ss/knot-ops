use thiserror::Error;
mod config;
mod transport;
mod daemon;
mod services;

pub use {config::ConfigError, transport::TransportError, daemon::DaemonError, services::ServiceError};

#[derive(Debug, Error)]
pub enum KnotError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Service(#[from] ServiceError)
}