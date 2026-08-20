use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use tokio::process::Command;
use tokio::time::Duration;
use tracing::{info, instrument, trace, warn};

use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::sys;
use crate::traits::ProcessControl;

/// Platform-specific process handle selected at compile time.
///
/// The concrete implementation depends on the target operating system:
///
/// - Linux: [`sys::linux::LinuxProcessHandle`]
/// - Windows: [`sys::windows::WindowsProcessHandle`]
/// - macOS: [`sys::macos::MacosProcessHandle`]
#[cfg(target_os = "linux")]
pub type PlatformHandle = sys::linux::LinuxProcessHandle;

#[cfg(windows)]
pub type PlatformHandle = sys::windows::WindowsProcessHandle;

#[cfg(target_os = "macos")]
pub type PlatformHandle = sys::macos::MacosProcessHandle;

/// Platform-independent process abstraction.
///
/// This is the primary process type exposed by the crate. It combines the
/// platform-specific [`ProcessControl`] implementation with the metadata
/// captured when the process was bound.
pub type Process = PlatformProcess<PlatformHandle>;

/// Owns a platform-specific process handle together with the metadata used to
/// identify the process instance.
///
/// `PlatformProcess` is responsible for the higher-level process lifecycle:
///
/// 1. obtaining process metadata;
/// 2. validating the expected process name;
/// 3. creating the platform-specific process handle;
/// 4. verifying that the process did not change between metadata snapshots;
/// 5. exposing process control operations through a platform-independent API.
///
/// The platform-specific [`ProcessControl`] implementation is responsible for
/// performing the actual operating-system operations.
#[derive(Debug)]
pub struct PlatformProcess<T: ProcessControl> {
    pub(crate) handle: T,
    pub(crate) metadata: ProcessMetadata,
}

impl<T: ProcessControl> PlatformProcess<T> {
    /// Returns the operating-system PID of the bound process.
    ///
    /// The PID is the value captured in [`ProcessMetadata`] when the process
    /// was successfully bound.
    pub fn pid(&self) -> u32 {
        self.metadata.pid
    }

    /// Compares an expected process name with the name reported by the
    /// operating system.
    ///
    /// The comparison is delegated to [`crate::utils::process_names_match`]
    /// so that platform-specific process-name differences are handled in one
    /// place.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Mismatch`] when the names do not match.
    async fn compare(expected_name: &str, actual_name: &str) -> Result<(), ProcessError> {
        if crate::utils::process_names_match(actual_name, expected_name) {
            Ok(())
        } else {
            Err(ProcessError::Mismatch {
                expected: expected_name.to_string(),
                actual: actual_name.to_string(),
            })
        }
    }

    /// Retrieves process metadata without blocking a Tokio worker thread.
    ///
    /// [`ProcessMetadata::extract`] performs synchronous process inspection.
    /// It is therefore executed through [`tokio::task::spawn_blocking`] before
    /// being exposed to the asynchronous process lifecycle.
    ///
    /// `io::ErrorKind::NotFound` is translated into
    /// [`ProcessError::NotRunning`] because a missing process is part of the
    /// process-control domain rather than an arbitrary I/O failure.
    async fn spawn_extract_metadata(pid: u32) -> Result<ProcessMetadata, ProcessError> {
        let metadata = tokio::task::spawn_blocking(move || ProcessMetadata::extract(pid))
            .await
            .map_err(|e| ProcessError::Io(io::Error::other(e)))?
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => ProcessError::NotRunning,
                _ => ProcessError::Io(e),
            })?;

        Ok(metadata)
    }

    /// Constructs a [`PlatformProcess`] from an already-created platform
    /// handle and its associated metadata.
    ///
    /// This constructor does not perform any additional validation. Callers
    /// are expected to provide metadata corresponding to the supplied handle.
    pub fn new(handle: T, metadata: ProcessMetadata) -> Self {
        Self { handle, metadata }
    }

    /// Binds a process identified by PID and expected executable name.
    ///
    /// This double-read is intentional. A PID can be reused while the handle
    /// is being created. Comparing the metadata before and after `T::bind`
    /// detects that race and prevents the resulting `PlatformProcess` from
    /// being associated with an inconsistent process identity.
    ///
    /// The process name is also validated against `expected_name` before the
    /// platform handle is created.
    ///
    /// # Errors
    ///
    /// - [`ProcessError::NotRunning`] if the process disappears during binding.
    /// - [`ProcessError::Mismatch`] if the process name does not match or the
    ///   process metadata changes while binding.
    /// - Other [`ProcessError`] variants when platform-specific binding fails.
    #[instrument(
        skip_all,
        name = "process_bind",
        fields(
            expected = %expected_name,
            pid = pid,
        )
    )]
    pub async fn bind(pid: u32, expected_name: String) -> Result<Self, ProcessError> {
        let metadata = Self::spawn_extract_metadata(pid).await?;
        Self::compare(&expected_name, &metadata.name).await?;

        let handle = T::bind(metadata.clone())?;
        let metadata_after = Self::spawn_extract_metadata(pid).await?;

        if metadata == metadata_after {
            Ok(Self::new(handle, metadata_after))
        } else {
            Err(ProcessError::Mismatch {
                expected: expected_name,
                actual: metadata_after.name,
            })
        }
    }

    /// Binds a process shortly after it has been spawned, retrying transient
    /// process-state races.
    ///
    /// A newly spawned process may not immediately expose stable metadata to
    /// the process-inspection API. Additionally, the process may terminate
    /// during the binding window.
    ///
    /// This method retries [`Self::bind`] up to 20 times with a 5 ms delay
    /// between attempts when the failure is [`ProcessError::Mismatch`] or
    /// [`ProcessError::NotRunning`].
    ///
    /// Other errors are considered non-transient and are returned immediately.
    async fn bind_after_spawn(pid: u32, expected_name: String) -> Result<Self, ProcessError> {
        const MAX_ATTEMPTS: u32 = 20;
        const RETRY_DELAY: Duration = Duration::from_millis(5);

        let mut last_err = None;

        for attempt in 0..MAX_ATTEMPTS {
            match Self::bind(pid, expected_name.clone()).await {
                Ok(process) => return Ok(process),
                Err(e @ ProcessError::Mismatch { .. }) | Err(e @ ProcessError::NotRunning) => {
                    last_err = Some(e);

                    if attempt + 1 < MAX_ATTEMPTS {
                        tokio::time::sleep(RETRY_DELAY).await;
                        continue;
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // At least one retryable error must have occurred before the loop
        // can exhaust all attempts.
        Err(last_err.expect("loop always sets last_err before exhausting attempts"))
    }

    /// Spawns a process without command-line arguments and binds it to a
    /// [`PlatformProcess`].
    ///
    /// This is equivalent to calling [`Self::spawn_with_args`] with an empty
    /// argument list.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError`] if the executable cannot be spawned or the
    /// resulting process cannot be successfully bound.
    pub async fn spawn(binary: &Path) -> Result<Self, ProcessError> {
        let args: [String; 0] = [];
        Self::spawn_with_args(binary, &args).await
    }

    /// Spawns an executable with the supplied arguments and binds the
    /// resulting process.
    ///
    /// Standard input, output, and error are disconnected from the child by
    /// redirecting all three streams to `null`.
    ///
    /// After the child is spawned, its PID is obtained and the child is
    /// monitored in a detached Tokio task. The actual process binding is then
    /// performed through [`Self::bind_after_spawn`], which retries transient
    /// races during process startup.
    ///
    /// The executable's file name is used as the expected process name when
    /// binding. This means successful return guarantees that the process
    /// discovered at the spawned PID matches the executable name according to
    /// [`crate::utils::process_names_match`].
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Io`] if:
    ///
    /// - the executable path does not contain a file name;
    /// - the process cannot be spawned;
    /// - the child PID cannot be obtained.
    ///
    /// Returns other [`ProcessError`] variants if binding the spawned process
    /// fails.
    #[instrument(
        skip_all,
        name = "process_spawn",
        fields(
            bin = %binary.display(),
        )
    )]
    pub async fn spawn_with_args(
        binary: &Path,
        args: &[impl AsRef<OsStr>],
    ) -> Result<Self, ProcessError> {
        trace!("spawning process...");

        let binary_name = binary
            .file_name()
            .ok_or_else(|| {
                io::Error::other(format!(
                    "failed to get file name: {}",
                    binary.to_string_lossy()
                ))
            })?
            .to_string_lossy()
            .to_string();

        let mut child = Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let id = child
            .id()
            .ok_or_else(|| io::Error::other("Failed to get child PID"))?;

        info!("process successfully spawned with PID: {}", id);

        // Keep the child process reaped even though ownership of the process
        // control handle is transferred to PlatformProcess.
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => trace!("process {} exited with status: {}", id, status),
                Err(e) => warn!("failed to wait for process {}: {}", id, e),
            }
        });

        Self::bind_after_spawn(id, binary_name).await
    }

    /// Forcefully terminates the process and waits for it to exit.
    ///
    /// The operation first verifies that the required process-control
    /// permissions are available, then issues the platform-specific forceful
    /// termination request and waits for process termination for at most
    /// `timeout`.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the process exited within the timeout.
    /// - `Ok(false)` if the process was still running when the timeout expired.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcessError`] if permission checking, termination, or
    /// waiting fails.
    pub async fn kill(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.kill()?;
        self.handle.wait(timeout).await
    }

    /// Requests process termination and waits for the process to exit.
    ///
    /// The termination mechanism is platform-specific and follows the
    /// semantics of [`ProcessControl::terminate`]. On platforms that support
    /// graceful termination, this may allow the process to perform cleanup;
    /// other platforms may implement it as forceful termination.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the process exited within the timeout.
    /// - `Ok(false)` if the process was still running when the timeout expired.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcessError`] if permission checking, termination, or
    /// waiting fails.
    pub async fn terminate(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.terminate()?;
        self.handle.wait(timeout).await
    }

    /// Waits for the process to exit without sending a termination signal.
    ///
    /// This method first verifies that the process-control operation is
    /// permitted and then delegates the actual wait to the platform-specific
    /// implementation.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the process exited within the timeout.
    /// - `Ok(false)` if the process is still running when the timeout expires.
    ///
    /// # Errors
    ///
    /// Returns a [`ProcessError`] if permission checking or waiting fails.
    pub async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.wait(timeout).await
    }
}
