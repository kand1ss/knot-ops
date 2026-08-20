use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND, GetLastError, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, TerminateProcess,
};

/// Maps Windows process-related errors to the crate's process-level error
/// abstraction.
///
/// `ERROR_INVALID_PARAMETER` and `ERROR_NOT_FOUND` are treated as indications
/// that the target process is no longer available and are therefore mapped to
/// [`ProcessError::NotRunning`].
///
/// All other operating-system errors are preserved as [`ProcessError::Io`].
pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(code) if code == ERROR_INVALID_PARAMETER as i32 || code == ERROR_NOT_FOUND as i32 => {
            ProcessError::NotRunning
        }
        _ => ProcessError::Io(error),
    }
}

/// Windows implementation of [`ProcessControl`] backed by an owned process
/// handle.
///
/// The handle is opened with the permissions required by the operations
/// implemented by this type:
///
/// - [`PROCESS_TERMINATE`] for process termination;
/// - [`PROCESS_QUERY_LIMITED_INFORMATION`] for checking process state;
/// - [`PROCESS_SYNCHRONIZE`] for waiting until the process exits.
///
/// [`OwnedHandle`] provides RAII ownership of the native Windows handle and
/// closes it automatically when the [`WindowsProcessHandle`] is dropped.
#[derive(Debug)]
pub struct WindowsProcessHandle {
    pub(crate) handle: OwnedHandle,
}

impl WindowsProcessHandle {
    /// Checks whether the referenced process has exited.
    ///
    /// `GetExitCodeProcess` returns [`STILL_ACTIVE`] while the process is
    /// running. Any other exit code indicates that the process has terminated.
    ///
    /// # Errors
    ///
    /// Returns the underlying Windows error if `GetExitCodeProcess` fails.
    fn is_process_exited(&self) -> io::Result<bool> {
        let mut exit_code = 0u32;

        let result = unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut exit_code) };

        if result == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(exit_code != STILL_ACTIVE as u32)
    }
}

#[async_trait::async_trait]
impl ProcessControl for WindowsProcessHandle {
    /// Opens a native Windows process handle for the process identified by
    /// the PID contained in `metadata`.
    ///
    /// The handle is opened with the minimum set of access rights required by
    /// this implementation: termination, limited process information queries,
    /// and synchronization.
    ///
    /// Once the handle has been successfully opened, subsequent operations
    /// use the handle rather than repeatedly resolving the PID.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] when Windows reports that the
    /// target process cannot be found.
    ///
    /// Other failures from `OpenProcess` are returned as
    /// [`ProcessError::Io`].
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
        let raw = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                metadata.pid,
            )
        };

        if raw.is_null() {
            let error = unsafe { GetLastError() };
            return Err(map_signal_error(io::Error::from_raw_os_error(error as i32)));
        }

        Ok(Self {
            handle: unsafe { OwnedHandle::from_raw_handle(raw.cast()) },
        })
    }

    /// Forcefully terminates the referenced process.
    ///
    /// Windows' [`TerminateProcess`] does not provide the same graceful
    /// termination semantics as a POSIX `SIGTERM`. It immediately terminates
    /// the target process with the supplied exit code (`1` in this
    /// implementation).
    ///
    /// If `TerminateProcess` reports failure, the implementation performs a
    /// second state check using [`Self::is_process_exited`]. This handles the
    /// race where the process exits concurrently with the termination request:
    /// the process may already be gone even though the termination call itself
    /// failed.
    ///
    /// This makes `kill()` effectively idempotent with respect to a process
    /// that has already exited.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Io`] if termination fails and the process is
    /// still running.
    ///
    /// Returns the state-checking error as [`ProcessError::Io`] if the process
    /// state cannot be determined after a failed termination request.
    fn kill(&self) -> Result<(), ProcessError> {
        let raw = self.handle.as_raw_handle();

        let result = unsafe { TerminateProcess(raw, 1) };
        if result != 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();

        if self.is_process_exited().map_err(ProcessError::Io)? {
            return Ok(());
        }

        Err(ProcessError::Io(error))
    }

    /// Terminates the referenced process.
    ///
    /// Windows does not expose a direct equivalent of POSIX `SIGTERM` through
    /// the API used by this implementation. Consequently, `terminate()` has
    /// the same forceful semantics as [`Self::kill`].
    ///
    /// Callers that require a distinction between graceful and forceful
    /// termination should implement that policy at a higher abstraction layer
    /// rather than relying on this method to provide graceful shutdown.
    fn terminate(&self) -> Result<(), ProcessError> {
        self.kill()
    }

    /// Waits until the referenced process exits or the timeout expires.
    ///
    /// Windows process handles can be used as synchronization objects:
    /// [`WaitForSingleObject`] becomes signaled when the associated process
    /// terminates.
    ///
    /// Because `WaitForSingleObject` is a blocking system call, it cannot be
    /// executed directly on a Tokio asynchronous worker thread. The operation
    /// is therefore executed inside [`tokio::task::spawn_blocking`].
    ///
    /// The handle is cloned before entering the blocking task so that the
    /// spawned operation owns its own handle reference. This ensures that the
    /// handle remains valid for the duration of the blocking wait independently
    /// of the lifetime of the original `WindowsProcessHandle`.
    ///
    /// The requested [`Duration`] is converted to milliseconds. Values larger
    /// than the maximum timeout representable by the Windows API are clamped
    /// to [`u32::MAX`].
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the process exited before the timeout expired.
    /// - `Ok(false)` if the timeout elapsed before the process exited.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Io`] if `WaitForSingleObject` fails or produces
    /// an unexpected result.
    ///
    /// The Tokio task's join failure is also converted into
    /// [`ProcessError::Io`].
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        let handle = self.handle.try_clone()?;

        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;

        tokio::task::spawn_blocking(move || {
            let result = unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    handle.as_raw_handle(),
                    timeout_ms,
                )
            };

            match result {
                windows_sys::Win32::Foundation::WAIT_OBJECT_0 => Ok(true),
                windows_sys::Win32::Foundation::WAIT_TIMEOUT => Ok(false),
                windows_sys::Win32::Foundation::WAIT_FAILED => {
                    Err(io::Error::last_os_error().into())
                }

                _ => Err(io::Error::other("unexpected WaitForSingleObject result").into()),
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    /// Checks whether the current process has permission to operate on the
    /// referenced process.
    ///
    /// No additional permission check is required here because
    /// [`Self::bind`] already opened the process handle with the access rights
    /// required by this implementation.
    ///
    /// A successful [`Self::bind`] therefore establishes that the requested
    /// process handle could be acquired with the required permissions.
    fn check_permissions(&self) -> Result<(), ProcessError> {
        Ok(())
    }
}
