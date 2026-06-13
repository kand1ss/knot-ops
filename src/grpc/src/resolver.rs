use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name, ToFsName, ToNsName};
use knot_core::errors::TransportError;
use std::path::Path;

/// Converts a filesystem path into a platform-specific IPC socket name.
///
/// This function acts as an abstraction layer to handle the differences between
/// Unix Domain Sockets (UDS) and Windows Named Pipes, ensuring that the provided
/// path is valid for the target operating system's IPC mechanism.
///
/// # Platform-Specific Logic
/// - **Unix:** Validates that the path length does not exceed 100 characters (a hard
///   limit for `sockaddr_un` on most Unix-like systems). It then converts the path
///   into a filesystem-based socket name.
/// - **Windows:** Extracts the filename component from the path and converts it into
///   a `GenericNamespaced` pipe name. This effectively ignores directory prefixes,
///   mapping paths like `C:\pipes\my.sock` to `\\.\pipe\my.sock`.
///
/// # Arguments
/// * `path` - The target path where the IPC socket should be created or connected to.
///
/// # Errors
/// Returns `TransportError::InvalidSocketPath` if:
/// - The path is empty.
/// - On Unix, the path length exceeds 100 characters.
/// - On Windows, the path does not contain a valid filename (e.g., a root drive).
/// - The underlying `interprocess` library fails to convert the path string into a valid socket name.
pub fn resolve_socket_name(path: &Path) -> Result<Name<'static>, TransportError> {
    if path.as_os_str().is_empty() {
        return Err(TransportError::InvalidSocketPath {
            path: path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    if path.as_os_str().len() > 100 {
        return Err(TransportError::InvalidSocketPath {
            path: path.to_path_buf(),
        });
    }

    if cfg!(windows) {
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or(TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })?
            .to_ns_name::<GenericNamespaced>()
            .map(|name| name.into_owned())
            .map_err(|_e| TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })
    } else {
        path.to_path_buf()
            .to_fs_name::<GenericFilePath>()
            .map_err(|_e| TransportError::InvalidSocketPath {
                path: path.to_path_buf(),
            })
    }
}

// Unit tests for `resolve_socket_name`
//
// Structure:
//   - `cross_platform::*` — cross-OS path resolving behavior (Unix paths on Windows & vice versa)
//   - `unix_tests::*`     — Unix-specific edge cases (length boundaries, sockaddr_un limits)
//   - `windows_tests::*`  — Windows-specific edge cases (filename extraction, root paths, parents)

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Verifies that an empty path always results in an error regardless of the OS.
    #[test]
    fn fails_on_empty_path() {
        let path = Path::new("");
        let result = resolve_socket_name(path);

        assert!(
            matches!(result, Err(TransportError::InvalidSocketPath { .. })),
            "Should return Err when the path is empty"
        );
    }

    /// Verifies that a Unix-style path string (forward slashes) can be correctly
    /// resolved on Windows (extracting the filename component).
    #[test]
    #[cfg(windows)]
    fn windows_resolves_unix_style_path_string() {
        // Hardcoded Unix path evaluated on a Windows target
        let path = Path::new("/tmp/deeply/nested/knot_daemon.sock");
        let result = resolve_socket_name(path);

        assert!(
            result.is_ok(),
            "Windows should successfully resolve a forward-slash Unix path string"
        );

        // Ensure that Windows extracted just the filename as the pipe name
        let name = result.unwrap();
        let s = format!("{:?}", name);
        assert!(
            s.contains("knot_daemon.sock"),
            "Name should contain the filename"
        );
    }

    /// Verifies that a Windows-style path string (backslashes) can be correctly
    /// resolved on Unix (treating the whole string as a valid file path).
    #[test]
    #[cfg(unix)]
    fn unix_resolves_windows_style_path_string() {
        // Hardcoded Windows path evaluated on a Unix target
        let path = Path::new(r"C:\ProgramData\Knot\knot.sock");
        let result = resolve_socket_name(path);

        assert!(
            result.is_ok(),
            "Unix should successfully resolve a backslash Windows path string"
        );

        // Ensure that Unix preserves the path character mapping exactly as required by fs_name
        let name = result.unwrap();
        assert!(name.to_str().unwrap().contains("knot.sock"));
    }

    #[cfg(unix)]
    mod unix_tests {
        use super::*;

        #[test]
        fn succeeds_on_valid_unix_path() {
            let path = Path::new("/tmp/knot_daemon.sock");
            let result = resolve_socket_name(path);

            assert!(
                result.is_ok(),
                "A valid absolute Unix path should resolve successfully"
            );
        }

        #[test]
        fn fails_when_path_exceeds_100_chars() {
            // Create a string that is exactly 101 characters long
            let long_name = "a".repeat(101);
            let path = Path::new(&long_name);
            let result = resolve_socket_name(path);

            assert!(
                matches!(result, Err(TransportError::InvalidSocketPath { .. })),
                "Paths longer than 100 characters must fail on Unix due to sockaddr_un limits"
            );
        }

        #[test]
        fn succeeds_when_path_is_exactly_100_chars() {
            // Boundary condition check (exactly 100 characters)
            let exact_name = "a".repeat(100);
            let path = Path::new(&exact_name);
            let result = resolve_socket_name(path);

            assert!(
                result.is_ok(),
                "A path that is exactly 100 characters long should be valid"
            );
        }
    }

    #[cfg(windows)]
    mod windows_tests {
        use super::*;

        #[test]
        fn succeeds_on_valid_windows_path() {
            let path = Path::new("C:\\temp\\knot.sock");
            let result = resolve_socket_name(path);

            assert!(
                result.is_ok(),
                "A valid absolute Windows path should resolve successfully"
            );

            let name = result.unwrap();
            let s = format!("{:?}", name);
            assert!(s.contains("knot.sock"), "Name should contain the filename");
        }

        #[test]
        fn succeeds_on_filename_only() {
            let path = Path::new("knot.sock");
            let result = resolve_socket_name(path);

            assert!(
                result.is_ok(),
                "A standalone filename should be valid for a Named Pipe name"
            );
        }

        #[test]
        fn fails_when_no_filename_can_be_extracted() {
            // Path::new("C:\\").file_name() returns None
            let path = Path::new("C:\\");
            let result = resolve_socket_name(path);

            assert!(
                matches!(result, Err(TransportError::InvalidSocketPath { .. })),
                "A path without a filename (e.g., drive root) must return an error"
            );
        }

        #[test]
        fn fails_on_path_ending_in_parent_dir() {
            // Path::new("C:\\temp\\..").file_name() returns None
            let path = Path::new("C:\\temp\\..");
            let result = resolve_socket_name(path);

            assert!(
                matches!(result, Err(TransportError::InvalidSocketPath { .. })),
                "Paths ending in '..' do not have a valid filename component and must fail"
            );
        }
    }
}
