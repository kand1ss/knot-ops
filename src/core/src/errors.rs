use thiserror::Error;
mod client;
mod config;
mod daemon;
mod services;
mod transport;

pub use {
    client::ClientError, config::ConfigError, daemon::DaemonError, services::ServiceError,
    transport::TransportError,
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
    #[error(transparent)]
    Client(#[from] ClientError),
}
