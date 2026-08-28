use crate::errors::PathResolutionError;
use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "knot";
const BIN_DIR: &str = "bin";

#[cfg(windows)]
pub const KNOT_CLI_BINARY_NAME: &str = "knot.exe";
#[cfg(not(windows))]
pub const KNOT_CLI_BINARY_NAME: &str = "knot";

#[cfg(windows)]
pub const KNOT_DAEMON_BINARY_NAME: &str = "knotd.exe";
#[cfg(not(windows))]
pub const KNOT_DAEMON_BINARY_NAME: &str = "knotd";

/// Standard per-user binary directory for Knot-managed executables.
///
/// This resolves through `directories::ProjectDirs`, so it follows the native
/// user data location on Windows, Linux, and macOS instead of relying on
/// privileged install directories.
pub fn binary_dir() -> Result<PathBuf, PathResolutionError> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().join(BIN_DIR))
        .ok_or(PathResolutionError)
}

pub fn cli_binary_path() -> Result<PathBuf, PathResolutionError> {
    binary_dir().map(|dir| dir.join(KNOT_CLI_BINARY_NAME))
}

pub fn daemon_binary_path() -> Result<PathBuf, PathResolutionError> {
    binary_dir().map(|dir| dir.join(KNOT_DAEMON_BINARY_NAME))
}

pub fn daemon_runtime_dir() -> PathBuf {
    #[cfg(not(windows))]
    {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(runtime_dir).join("knot");
        }

        // Fallback: derive the runtime directory from the current UID.
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/run/user/{uid}/knot"))
    }

    #[cfg(windows)]
    {
        PathBuf::from(r"C:\Windows\Temp\knot")
    }
}
