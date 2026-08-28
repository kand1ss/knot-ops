use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;
use tokio::io::unix::AsyncFd;

/// A reference to a Linux process represented by an owned `pidfd`.
///
/// A `pidfd` identifies a specific process instance and remains associated with
/// that process even if the kernel later reuses its numeric PID. This
/// makes it suitable for safely tracking a process after it has been bound.
type ProcessRef = AsyncFd<OwnedFd>;

/// Maps errors returned by Linux process-related syscalls to the crate's
/// process-level error abstraction.
///
/// `ESRCH` indicates that the referenced process no longer exists. At the
/// abstraction level of [`ProcessControl`], this is represented as
/// [`ProcessError::NotRunning`] rather than as a generic I/O error.
///
/// All other operating-system errors are preserved as [`ProcessError::Io`].
pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(libc::ESRCH) => ProcessError::NotRunning,
        _ => ProcessError::Io(error),
    }
}

/// Sends a signal to the process referenced by `pid_fd`.
///
/// This function uses the Linux `pidfd_send_signal(2)` syscall rather than
/// sending a signal to a numeric PID directly. As a result, the signal is
/// delivered to the specific process represented by the `pidfd`, avoiding PID
/// reuse races.
///
/// # Errors
///
/// Returns [`ProcessError::NotRunning`] if the process no longer exists.
/// Other syscall failures are returned as [`ProcessError::Io`].
pub fn send_signal(pid_fd: &AsyncFd<OwnedFd>, signal: libc::c_int) -> Result<(), ProcessError> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pid_fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };

    if result == -1 {
        return Err(map_signal_error(io::Error::last_os_error()));
    }

    Ok(())
}

/// Linux implementation of [`ProcessControl`] backed by a `pidfd`.
///
/// The handle owns the underlying file descriptor, so the kernel reference to
/// the process is kept alive for the lifetime of this value.
///
/// Unlike a raw PID, a `pidfd` refers to the specific process instance that was
/// opened by [`LinuxProcessHandle::bind`]. This prevents operations such as
/// `kill()` or `wait()` from accidentally targeting a different process after
/// PID reuse.
///
/// The underlying descriptor is also registered with Tokio's [`AsyncFd`],
/// allowing [`LinuxProcessHandle::wait`] to asynchronously wait for process
/// termination without polling at a fixed interval.
#[derive(Debug)]
pub struct LinuxProcessHandle {
    pub(crate) process_ref: ProcessRef,
}

impl LinuxProcessHandle {
    /// Checks whether the process referenced by the `pidfd` has already exited.
    ///
    /// Linux makes a `pidfd` readable when the referenced process terminates.
    /// This method performs a non-blocking `poll(2)` with a zero timeout and
    /// therefore never waits for the process.
    ///
    /// The method is used both for zero-timeout waits and to verify that an
    /// `AsyncFd` readiness notification still corresponds to actual process
    /// termination.
    fn poll_readable_now(&self) -> io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.process_ref.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        match unsafe { libc::poll(&mut pfd, 1, 0) } {
            -1 => Err(io::Error::last_os_error()),
            0 => Ok(false),
            _ => Ok(pfd.revents & libc::POLLIN != 0),
        }
    }
}

#[async_trait::async_trait]
impl ProcessControl for LinuxProcessHandle {
    /// Binds a process handle to the process identified by the supplied
    /// [`ProcessMetadata`].
    ///
    /// This opens a Linux `pidfd` for the process's PID. Once opened, all
    /// following process operations are performed through the `pidfd` rather
    /// than through the numeric PID itself.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the process does not exist.
    /// Other failures of `pidfd_open(2)` are returned as
    /// [`ProcessError::Io`].
    ///
    /// # Safety
    ///
    /// `pidfd_open` returns a newly allocated file descriptor on success.
    /// The descriptor is immediately transferred into [`OwnedFd`], making its
    /// ownership explicit and ensuring that it is closed when the handle is
    /// dropped.
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
        let pid = metadata.pid;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };

        if fd < 0 {
            return Err(map_signal_error(io::Error::last_os_error()));
        }

        let fd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
        Ok(Self {
            process_ref: ProcessRef::new(fd)?,
        })
    }

    /// Immediately terminates the referenced process using `SIGKILL`.
    ///
    /// `SIGKILL` cannot be caught or ignored by the target process. The signal
    /// is sent through the `pidfd`, so PID reuse cannot cause the signal to be
    /// delivered to an unrelated process.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the process has already exited.
    /// Other failures are propagated as [`ProcessError::Io`].
    fn kill(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, libc::SIGKILL)?;
        Ok(())
    }

    /// Requests graceful termination of the referenced process using `SIGTERM`.
    ///
    /// Unlike [`Self::kill`], `SIGTERM` can be caught or ignored by the target
    /// process. Callers that require confirmation of termination should follow
    /// this operation with [`Self::wait`].
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the process has already exited.
    /// Other failures are propagated as [`ProcessError::Io`].
    fn terminate(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, libc::SIGTERM)?;
        Ok(())
    }

    /// Waits until the referenced process exits or the timeout expires.
    ///
    /// Linux exposes process termination through the readiness state of a
    /// `pidfd`. Tokio's [`AsyncFd`] is used to wait for this readiness event
    /// without blocking the executor thread.
    ///
    /// A zero-duration timeout performs an immediate, non-blocking check and
    /// returns `true` only if the process has already exited.
    ///
    /// For a non-zero timeout, the method waits asynchronously until either:
    ///
    /// - the `pidfd` becomes readable, indicating process termination; or
    /// - the supplied timeout expires.
    ///
    /// The readiness notification is verified with [`Self::poll_readable_now`]
    /// because an `AsyncFd` readiness event does not by itself guarantee that
    /// the descriptor remains ready when the readiness guard is acquired.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the process has exited.
    /// - `Ok(false)` if the timeout elapsed before termination was observed.
    ///
    /// # Errors
    ///
    /// Returns an I/O-related [`ProcessError`] if checking the descriptor or
    /// waiting for readiness fails.
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        if timeout.is_zero() {
            return self.poll_readable_now().map_err(map_signal_error);
        }

        let wait_for_exit = async {
            loop {
                let mut guard = self.process_ref.readable().await?;
                let really_ready = guard.try_io(|_| {
                    if self.poll_readable_now()? {
                        Ok(())
                    } else {
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    }
                });

                match really_ready {
                    Ok(result) => {
                        result?;
                        return Ok(true);
                    }
                    Err(_would_block) => continue,
                }
            }
        };

        match tokio::time::timeout(timeout, wait_for_exit).await {
            Ok(result) => result.map_err(map_signal_error),
            Err(_elapsed) => Ok(false),
        }
    }

    /// Checks whether the current process has permission to signal the
    /// referenced process.
    ///
    /// Linux defines signal `0` as a permission/existence check: no signal is
    /// actually delivered, but the kernel performs the same permission checks
    /// that would be performed for a real signal.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the process no longer exists.
    /// Permission failures and other syscall errors are returned as
    /// [`ProcessError::Io`].
    fn check_permissions(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, 0)?;
        Ok(())
    }
}
