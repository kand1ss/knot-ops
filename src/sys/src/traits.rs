use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use std::fmt::Debug;
use tokio::time::Duration;

/// Provides platform-independent control over a specific operating-system
/// process.
///
/// Implementations encapsulate the platform-specific mechanism used to
/// identify, signal, and wait for a process while exposing a common contract
/// to the rest of the crate.
///
/// A [`ProcessControl`] implementation represents a process instance rather
/// than merely a numeric PID. Platform implementations are responsible for
/// preventing operations from being accidentally applied to a different
/// process after PID reuse.
///
/// # Platform implementations
///
/// The underlying mechanism differs by operating system:
///
/// - Linux uses a `pidfd` to maintain a stable reference to the process.
/// - Windows uses an owned native process handle.
/// - macOS stores process metadata and verifies the process identity before
///   PID-based operations.
///
/// The platform-specific implementation must preserve the semantics defined
/// by this trait even when the underlying operating-system APIs differ.
#[async_trait::async_trait]
pub trait ProcessControl: Debug + Sized {
    /// Binds a process control handle to the process described by `metadata`.
    ///
    /// The supplied metadata identifies the process instance that subsequent
    /// operations are expected to target. Implementations may acquire a
    /// platform-specific process handle or retain the metadata for later
    /// identity verification.
    ///
    /// A successful call establishes the process reference represented by the
    /// returned handle.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the target process does not
    /// exist at bind time.
    ///
    /// Other platform-specific failures are returned through
    /// [`ProcessError`].
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError>;

    /// Immediately terminates the bound process using the platform's forceful
    /// termination mechanism.
    ///
    /// This operation is intended for cases where the process must be stopped
    /// regardless of whether it can handle or reject a graceful termination
    /// request.
    ///
    /// Implementations should ensure that the operation targets the process
    /// instance originally bound to the handle and cannot accidentally target
    /// an unrelated process after PID reuse.
    ///
    /// The method does not imply that the process has already exited when it
    /// returns successfully. Callers that require confirmation of termination
    /// should use [`Self::wait`] afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the bound process is no longer
    /// running.
    ///
    /// Returns [`ProcessError::Reused`] when the original process identity can
    /// no longer be associated with the stored process identifier.
    ///
    /// Other platform-specific failures are returned through
    /// [`ProcessError`].
    fn kill(&self) -> Result<(), ProcessError>;

    /// Requests graceful termination of the bound process.
    ///
    /// Unlike [`Self::kill`], this operation should use the platform mechanism
    /// intended for a cooperative termination request when such a mechanism is
    /// available.
    ///
    /// Graceful termination is inherently platform-dependent. For example,
    /// POSIX implementations can use `SIGTERM`, while Windows may require a
    /// different mechanism or may provide only forceful termination through
    /// the underlying API.
    ///
    /// Implementations that cannot provide a distinct graceful-termination
    /// mechanism may legitimately implement this operation using the same
    /// mechanism as [`Self::kill`].
    ///
    /// Successful return means that the termination request was accepted; it
    /// does not guarantee that the process has already exited. Use
    /// [`Self::wait`] when termination must be confirmed.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the bound process is no longer
    /// running.
    ///
    /// Returns [`ProcessError::Reused`] when the original process identity can
    /// no longer be associated with the stored process identifier.
    ///
    /// Other platform-specific failures are returned through
    /// [`ProcessError`].
    fn terminate(&self) -> Result<(), ProcessError>;

    /// Waits for the bound process to exit, up to the specified timeout.
    ///
    /// This method is asynchronous and must not block the Tokio runtime's
    /// asynchronous worker threads.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the bound process has exited before the timeout expires.
    /// - `Ok(false)` if the timeout expires while the process is still running.
    ///
    /// A zero-duration timeout represents a non-blocking state check and
    /// should return immediately.
    ///
    /// The timeout is a maximum wait duration; implementations should not
    /// intentionally extend it because of internal polling or scheduling
    /// delays.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Reused`] if the implementation detects that the
    /// original process identity has been replaced by another process using
    /// the same PID.
    ///
    /// Other platform-specific failures are returned through
    /// [`ProcessError`].
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError>;

    /// Verifies that the current process handle can be used for process
    /// operations.
    ///
    /// This check is intended to validate that the caller has the permissions
    /// required to perform process-control operations on the bound process.
    ///
    /// Implementations may use a platform-specific no-op or rely on
    /// permissions already established while binding the process when the
    /// operating system does not provide a separate permission-checking
    /// primitive.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotRunning`] if the bound process no longer
    /// exists.
    ///
    /// Returns [`ProcessError::Reused`] if the process identifier now refers
    /// to a different process.
    ///
    /// Returns a platform-specific error if the required permissions are not
    /// available.
    fn check_permissions(&self) -> Result<(), ProcessError>;
}
