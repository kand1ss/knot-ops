use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::traits::ProcessControl;
use std::io;
use std::time::Duration;

/// Maps macOS process-related errors to the crate's process-level error
/// abstraction.
///
/// `ESRCH` indicates that the process identified by the supplied PID does not
/// currently exist and is therefore mapped to [`ProcessError::NotRunning`].
///
/// All other operating-system errors are preserved as [`ProcessError::Io`].
pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(libc::ESRCH) => ProcessError::NotRunning,
        _ => ProcessError::Io(error),
    }
}

/// macOS implementation of [`ProcessControl`].
///
/// macOS does not provide the Linux `pidfd` mechanism used to bind operations
/// to a specific process instance. This implementation therefore stores the
/// [`ProcessMetadata`] observed when the handle is created and uses it as a
/// process fingerprint.
///
/// Before performing operations that target the process, the current metadata
/// for the stored PID is compared with the original fingerprint. If the
/// metadata differs, the original process has exited and the PID has been
/// reused by another process, resulting in [`ProcessError::Reused`].
#[derive(Debug)]
pub struct MacosProcessHandle {
    pub(crate) fingerprint: ProcessMetadata,
}

impl MacosProcessHandle {
    /// Verifies that the PID still refers to the process originally bound to
    /// this handle.
    ///
    /// The current [`ProcessMetadata`] is extracted for the stored PID and
    /// compared with the fingerprint captured during [`Self::bind`].
    ///
    /// # Errors
    ///
    /// - Returns `Ok(())` if the PID still identifies the original process.
    /// - Returns [`ProcessError::Reused`] if the PID now belongs to a different
    ///   process.
    /// - Returns [`ProcessError::NotRunning`] if the original PID no longer
    ///   exists.
    /// - Returns [`ProcessError::Io`] for other metadata lookup failures.
    fn ensure_pid_not_reused(&self) -> Result<(), ProcessError> {
        match ProcessMetadata::extract(self.fingerprint.pid) {
            Ok(metadata) if metadata == self.fingerprint => Ok(()),

            Ok(_) => Err(ProcessError::Reused),

            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(ProcessError::NotRunning),

            Err(error) => Err(ProcessError::Io(error)),
        }
    }

    /// Checks whether the PID still refers to the process originally bound to
    /// this handle.
    ///
    /// Unlike [`Self::ensure_pid_not_reused`], this method treats a missing PID
    /// as a normal `false` result because process disappearance is the expected
    /// termination condition for [`Self::wait`].
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the original process is still running.
    /// - `Ok(false)` if the PID no longer exists.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the PID now belongs to a different
    /// process, or [`ProcessError::Io`] if the process metadata cannot be read.
    fn check_identity(&self) -> Result<bool, ProcessError> {
        match ProcessMetadata::extract(self.fingerprint.pid) {
            Ok(metadata) if metadata == self.fingerprint => Ok(true),
            Ok(_) => Err(ProcessError::Reused),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(ProcessError::Io(e)),
        }
    }
}

/// Sends a signal to the process identified by the supplied PID.
///
/// This is a thin wrapper around the POSIX `kill(2)` syscall. Unlike Linux
/// `pidfd_send_signal`, this operation targets a numeric PID directly, so
/// callers must verify process identity before invoking it when PID reuse is a
/// concern.
fn send_signal(pid: u32, signal: libc::c_int) -> io::Result<()> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };

    if result == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[async_trait::async_trait]
impl ProcessControl for MacosProcessHandle {
    /// Creates a process handle from the supplied process metadata.
    ///
    /// No operating-system process handle is opened because macOS does not
    /// provide an equivalent to Linux `pidfd` that is used by this
    /// implementation. Instead, the supplied metadata is retained as the
    /// process fingerprint and is used for subsequent PID reuse detection.
    ///
    /// The caller is therefore responsible for ensuring that `metadata`
    /// describes the intended process at bind time.
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
        Ok(Self {
            fingerprint: metadata.clone(),
        })
    }

    /// Forcefully terminates the bound process using `SIGKILL`.
    ///
    /// Before sending the signal, the process identity is verified against the
    /// stored fingerprint. This prevents a recycled PID from causing the
    /// signal to be delivered to an unrelated process.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the PID has been reused,
    /// [`ProcessError::NotRunning`] if the process no longer exists, or
    /// [`ProcessError::Io`] if the signal operation fails for another reason.
    fn kill(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;

        send_signal(self.fingerprint.pid, libc::SIGKILL).map_err(map_signal_error)?;

        Ok(())
    }

    /// Requests termination of the bound process using `SIGTERM`.
    ///
    /// `SIGTERM` allows the target process to handle or ignore the termination
    /// request, unlike [`Self::kill`], which uses `SIGKILL`.
    ///
    /// The process identity is verified before sending the signal to prevent a
    /// recycled PID from being targeted.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the PID has been reused,
    /// [`ProcessError::NotRunning`] if the process no longer exists, or
    /// [`ProcessError::Io`] if the signal operation fails for another reason.
    fn terminate(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;

        send_signal(self.fingerprint.pid, libc::SIGTERM).map_err(map_signal_error)?;

        Ok(())
    }

    /// Waits until the bound process exits or the timeout expires.
    ///
    /// macOS does not provide a process handle equivalent to Linux `pidfd` that
    /// can be used here for asynchronous termination notifications. This
    /// implementation therefore uses bounded polling of the process identity.
    ///
    /// The current process metadata is checked at most once every 100
    /// milliseconds. If the PID disappears, the process is considered
    /// terminated. If the PID is reused by another process, the operation
    /// fails with [`ProcessError::Reused`] rather than treating the unrelated
    /// process as the original one.
    ///
    /// The timeout is measured against a fixed deadline, so polling and task
    /// scheduling delays cannot extend the requested timeout indefinitely.
    ///
    /// Each metadata lookup is executed through [`tokio::task::spawn_blocking`]
    /// because process metadata retrieval is a blocking operating-system
    /// operation and must not block a Tokio asynchronous worker thread.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the original process has exited.
    /// - `Ok(false)` if the timeout expires while the original process is still
    ///   running.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the PID is observed to belong to a
    /// different process, or [`ProcessError::Io`] if metadata retrieval or
    /// execution of the blocking task fails.
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let fingerprint = self.fingerprint.clone();
            let exists = tokio::task::spawn_blocking(move || {
                let owned = MacosProcessHandle {
                    fingerprint: fingerprint.clone(),
                };
                owned.check_identity()
            })
            .await
            .map_err(|e| ProcessError::Io(io::Error::other(e)))??;

            if !exists {
                return Ok(true);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(false);
            }

            let remaining = deadline - now;
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
    }

    /// Verifies that the current process still refers to the originally bound
    /// process and that a signal with value `0` can be sent to it.
    ///
    /// POSIX signal `0` does not actually deliver a signal. It causes the
    /// kernel to perform the normal process existence and permission checks,
    /// making it suitable for checking whether the caller can signal the
    /// target process.
    ///
    /// The identity check is performed first to ensure that a reused PID is
    /// reported as [`ProcessError::Reused`] rather than accidentally checking
    /// permissions against an unrelated process.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the PID has been reused,
    /// [`ProcessError::NotRunning`] if the process no longer exists, or
    /// [`ProcessError::Io`] if the signal operation fails for another reason.
    fn check_permissions(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;
        send_signal(self.fingerprint.pid, 0).map_err(map_signal_error)?;
        Ok(())
    }
}
