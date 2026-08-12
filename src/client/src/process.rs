use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::path::Path;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use thiserror::Error;
use tokio::process::Command;
use tracing::{info, instrument, trace, warn};

/// A trait representing basic process control functionality.
///
/// This trait provides methods to manage and retrieve information
/// about a process, including the ability to terminate a process
/// and retrieve its Process ID (PID).
///
/// # Required Methods
///
/// - `kill`: Terminates the associated process.
/// - `pid`: Returns the Process ID (PID) of the associated process.
///
/// # Errors
///
/// The `kill` method may return an `std::io::Result::Err` if the
/// process cannot be terminated, e.g., due to insufficient permissions
/// or the process not existing.
///
/// # Required Traits
///
/// - `Debug`: Implementors of `ProcessControl` must also implement the
///   `Debug` trait, allowing for debugging representations of the process
///   control object.
pub trait ProcessControl: Debug {
    /// Terminates the associated process or operation represented by the implementing object.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the process or operation was successfully terminated.
    /// * `Err(std::io::Error)` if an error occurs during termination.
    ///
    /// # Errors
    /// This function may fail due to various I/O errors, such as insufficient permissions,
    /// the process no longer existing, or system-level errors.
    fn kill(&self) -> io::Result<()>;

    /// Retrieves the process identifier (PID).
    ///
    /// # Returns
    ///
    /// * `u32` - The process ID associated with the current instance.
    ///
    /// # Notes
    /// - The PID is typically used to identify and manage processes.
    /// - Ensure the instance is valid and properly initialized before calling this method.
    ///
    /// # Errors
    /// This function does not return an error, but the validity of the returned PID depends
    /// on the context in which it is called.
    fn pid(&self) -> u32;
}

/// Represents errors that can occur during a process-related operation.
///
/// # Variants
///
/// * `NotRunning`: Indicates that the process is not currently running.
///
/// * `Mismatch { expected, actual }`: Indicates that there is a mismatch
///   in the process name. It provides details about the expected process
///   name and the actual process name encountered during the operation.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process is not running")]
    NotRunning,

    #[error("process name mismatch: expected '{expected}', got '{actual}'")]
    Mismatch { expected: String, actual: String },
}

/// The `Process` struct represents an operating system process with a specific process identifier (PID).
///
/// # Fields
/// - `pid` (u32): The process identifier (PID) associated with this process.
///   - This field is visible only within the current crate (`pub(crate)` access modifier).
///
/// # Derives
/// - `Debug`: Automatically implements the `fmt::Debug` trait, enabling debug formatting for instances of `Process`.
#[derive(Debug)]
pub struct Process {
    pub(crate) pid: u32,
}

impl Process {
    fn new(pid: u32) -> Self {
        Self { pid }
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }

    #[cfg(windows)]
    fn normalize_process_name(name: &str) -> &str {
        name.strip_suffix(".exe")
            .or_else(|| name.strip_suffix(".EXE"))
            .unwrap_or(name)
    }

    #[cfg(not(windows))]
    fn normalize_process_name(name: &str) -> &str {
        name
    }

    /// Compares two process names for equality after normalizing them and ignoring case differences.
    ///
    /// This function takes two process names as input, normalizes them, and checks if they are equal
    /// in a case-insensitive manner. The normalization process is handled by the `normalize_process_name`
    /// method, which prepares the process names for comparison. The comparison itself is performed
    /// with ASCII case insensitivity.
    ///
    /// # Parameters
    /// - `actual`: A string slice representing the actual process name.
    /// - `expected`: A string slice representing the expected process name.
    ///
    /// # Returns
    /// - `true` if the normalized process names are equal, ignoring ASCII case differences.
    /// - `false` otherwise.
    fn process_names_match(actual: &str, expected: &str) -> bool {
        Self::normalize_process_name(actual)
            .eq_ignore_ascii_case(Self::normalize_process_name(expected))
    }

    /// Asynchronously binds to a process with the given process identifier (PID) and checks if its name matches
    /// the expected name. If successful, returns an instance of `Self`; otherwise, returns an appropriate `ProcessError`.
    ///
    /// # Arguments
    ///
    /// * `pid` - The process identifier (PID) of the target process as a `u32`.
    /// * `expected_name` - The expected name of the process as a `String`. The actual process name will be compared
    ///   to this value in a case-insensitive manner.
    ///
    /// # Returns
    ///
    /// * `Result<Self, ProcessError>`
    ///     - `Ok(Self)` if a process with the specified `pid` is running and its name matches the `expected_name`.
    ///     - `Err(ProcessError)` if the process is not running, the name doesn't match, or another error occurs.
    ///
    /// # Errors
    ///
    /// * `ProcessError::NotRunning` - If the process with the specified `pid` is not currently running, or a failure occurs while
    ///   attempting to access the process information.
    /// * `ProcessError::Mismatch` - If the running process name differs from the provided `expected_name`, wrapped with
    ///   the expected and actual names.
    ///
    /// # Implementation Details
    ///
    /// The function leverages the `sysinfo` crate to fetch and refresh the system's process list, focusing only on the
    /// process with the specified `pid`. The process name is compared against the `expected_name` in a case-insensitive
    /// manner. It uses `tokio::task::spawn_blocking` to offload the system calls to a blocking thread since process
    /// refreshing can be a blocking operation.
    pub async fn bind(pid: u32, expected_name: String) -> Result<Self, ProcessError> {
        let sys_pid = Pid::from(pid as usize);
        tokio::task::spawn_blocking(move || {
            let mut sys = System::new();

            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[sys_pid]),
                false,
                ProcessRefreshKind::nothing(),
            );

            match sys.process(sys_pid) {
                Some(process) => {
                    let actual_name = process.name().to_string_lossy();
                    if Self::process_names_match(&actual_name, &expected_name) {
                        Ok(Self::new(pid))
                    } else {
                        Err(ProcessError::Mismatch {
                            expected: expected_name,
                            actual: actual_name.to_string(),
                        })
                    }
                }
                None => Err(ProcessError::NotRunning),
            }
        })
        .await
        .map_err(|_| ProcessError::NotRunning)?
    }

    /// Spawns a new process for the given binary without any additional arguments.
    ///
    /// # Parameters
    /// - `binary`: A reference to a `Path` representing the executable to be run.
    ///
    /// # Returns
    /// - `io::Result<Self>`: Returns an instance of the implementing type if the
    ///   process is spawned successfully, or an `io::Error` if it fails.
    ///
    /// # Behavior
    /// This function calls `Self::spawn_with_args` with the provided binary and an empty
    /// array of arguments, effectively launching the binary without any command-line
    /// arguments.
    ///
    /// # Errors
    /// This function propagates any I/O errors that occur when attempting to spawn
    /// the process.
    pub fn spawn(binary: &Path) -> io::Result<Self> {
        let args: [String; 0] = [];
        Self::spawn_with_args(binary, &args)
    }

    /// Spawns a new process using the specified binary and arguments.
    ///
    /// This function facilitates the creation of a new child process, where the binary to be executed
    /// and its associated arguments are provided by the caller. The process is spawned with its standard
    /// input, output, and error streams disabled (set to /dev/null or equivalent).
    ///
    /// ## Instrumentation
    /// - Traces the process spawning action.
    /// - Logs the process ID (PID) on successful spawning, or a warning if the process exits immediately.
    ///
    /// ## Parameters
    /// - `binary`: A reference to a [`Path`] specifying the location of the executable binary to run.
    /// - `args`: A slice of items that can be converted to [`OsStr`], representing the arguments
    ///   to be passed to the binary during execution.
    ///
    /// ## Returns
    /// If the process successfully spawns, this function returns an `Ok(Self)` containing an initialized
    /// instance of the struct with the newly created process's PID. If the process fails to spawn or exits
    /// immediately after for any reason, it returns a [`std::io::Error`] wrapped in `Err`.
    ///
    /// ## Errors
    /// - Returns an error if the spawning of the process fails (e.g., permission errors, missing binary, etc.).
    /// - Returns an error if the process exits immediately without yielding a PID, providing a detailed
    ///   message including the binary's path.
    ///
    /// ## Notes
    /// - The `fields` specified in the `#[instrument]` macro (e.g., `bin`) allow for enhanced tracing of
    ///   process-related data, such as the displayed path of the binary being executed.
    /// - The standard streams (stdin, stdout, stderr) are explicitly set to null to ensure the detached running
    ///   of the spawned process.
    ///
    /// [`Path`]: https://doc.rust-lang.org/std/path/struct.Path.html
    /// [`OsStr`]: https://doc.rust-lang.org/std/ffi/struct.OsStr.html
    /// [`std::io::Error`]: https://doc.rust-lang.org/std/io/struct.Error.html
    #[instrument(
        skip_all,
        name = "process_spawn",
        fields(
            bin = %binary.display(),
        ))]
    pub fn spawn_with_args(binary: &Path, args: &[impl AsRef<OsStr>]) -> io::Result<Self> {
        trace!("spawning process...");
        let child = Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        match child.id() {
            Some(id) => {
                info!("process successfully spawned with PID: {}", id);
                Ok(Self::new(id))
            }
            None => {
                warn!("daemon process exited immediately after spawning.");
                Err(io::Error::other(format!(
                    "daemon process at '{}' exited immediately and yielded no PID. It might have crashed on startup.",
                    binary.to_string_lossy()
                )))
            }
        }
    }
}

impl ProcessControl for Process {
    #[cfg(windows)]
    fn kill(&self) -> io::Result<()> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, self.pid);
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let ok = TerminateProcess(handle, 1);
            CloseHandle(handle);
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    fn kill(&self) -> io::Result<()> {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        kill(Pid::from_raw(self.pid as i32), Signal::SIGKILL).map_err(io::Error::from)
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}
