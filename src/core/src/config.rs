use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration settings for an individual service.
///
/// Defines how a process should be executed, its environment,
/// and its relationship to other services.
pub struct ServiceConfig {
    /// The exact command or binary path to be executed.
    pub execution_command: String,
    /// The working directory where the process should be launched.
    pub directory: String,
    /// A map of environment variables to be passed to the process.
    pub env: HashMap<String, String>,
    /// A list of service names that must be started before this service can run.
    pub depends_on: Vec<String>,
}

/// The root configuration for the entire project.
///
/// Acts as the primary container for all managed services and
/// the global context of the project.
pub struct ProjectConfig {
    /// A collection of all service configurations defined in the project.
    pub services: Vec<ServiceConfig>,
    /// The absolute or relative path to the root directory of the project.
    pub project_directory: PathBuf,
}
