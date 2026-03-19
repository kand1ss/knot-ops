use std::collections::HashMap;
use std::path::Path;

pub struct ServiceConfig {
    pub execution_command: String,
    pub directory: String,
    pub env: HashMap<String, String>,
    pub depends_on: Vec<String>,
}

pub struct ProjectConfig {
    pub services: Vec<ServiceConfig>,
    pub project_directory: Path,
}
