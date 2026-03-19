use thiserror::Error;
use std::path::PathBuf;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config not found in '{path}' or any parent directory")]
    NotFound { path: PathBuf },

    #[error("not a knot project, run 'knot init' in project folder to initialize")]
    NotInitialized,

    #[error("failed to parse config at '{path}': {reason}")]
    ParseError {
        path:   PathBuf,
        reason: String,
    },

    #[error("invalid config: {reason}")]
    ValidationError { reason: String },

    #[error("service '{service}' depends on unknown service '{dependency}'")]
    UnknownDependency {
        service:    String,
        dependency: String,
    },

    #[error("circular dependency detected: {}", cycle.join(" → "))]
    CircularDependency { cycle: Vec<String> },

    #[error("group '{group}' references unknown service '{service}'")]
    UnknownServiceInGroup {
        group:   String,
        service: String,
    },

    #[error("failed to read config file '{path}': {source}")]
    ReadError {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write config file '{path}': {source}")]
    WriteError {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("already initialized, '{path}' already exists")]
    AlreadyInitialized { path: PathBuf },
}


