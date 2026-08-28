use thiserror::Error;

#[derive(Debug, Error)]
#[error("could not resolve a valid application data directory for this platform/user")]
pub struct PathResolutionError;
