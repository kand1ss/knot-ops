use std::collections::HashMap;
use std::path::Path;

pub struct ServiceConfig {
    execution_command: String,
    directory: String,
    env: HashMap<String, String>,
    depends_on: Vec<String>,
}

pub struct ProjectConfig {
    services: Vec<ServiceConfig>,
    project_directory: Path,
}
