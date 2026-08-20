use std::io;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Identifies a specific process instance.
///
/// The PID alone is not sufficient to identify a process over time because
/// operating systems may reuse PIDs after a process exits. `start_time` is
/// therefore included as part of the process identity and can be used to
/// distinguish the original process from a later process that received the
/// same PID.
///
/// `name` is retained as descriptive metadata and may be useful for validating
/// that the expected executable is associated with the process, but it should
/// not be treated as a globally unique process identifier.
///
/// # Identity
///
/// For process-instance identity, the combination of [`Self::pid`] and
/// [`Self::start_time`] is the significant fingerprint. If the PID remains
/// the same but the start time changes, the original process has exited and
/// the PID has been reused.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct ProcessMetadata {
    /// Operating-system process identifier.
    ///
    /// A PID is only unique while the corresponding process exists. After the
    /// process exits, the operating system may assign the same PID to another
    /// process.
    pub pid: u32,

    /// Process name reported by the operating system.
    ///
    /// This field is descriptive and is not sufficient by itself to identify a
    /// specific process instance because multiple processes may have the same
    /// name.
    pub name: String,

    /// Process start time as reported by [`sysinfo`].
    ///
    /// This value is used together with [`Self::pid`] to distinguish a process
    /// instance from a later process that reuses the same PID.
    pub start_time: u64,
}

impl ProcessMetadata {
    /// Extracts the current metadata for the process identified by `pid`.
    ///
    /// A fresh [`System`] snapshot is created and refreshed for the requested
    /// PID. Only the process existence and metadata required to construct
    /// [`ProcessMetadata`] are retrieved.
    ///
    /// This method performs a point-in-time observation. It does not keep the
    /// process alive, reserve the PID, or prevent the process from exiting or
    /// being replaced immediately after the metadata has been read.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] if no process with the supplied PID
    /// exists when the snapshot is taken.
    ///
    /// Other errors returned by the underlying process metadata operations are
    /// propagated as [`io::Error`].
    pub fn extract(pid: u32) -> io::Result<ProcessMetadata> {
        let sys_pid = Pid::from(pid as usize);
        let mut sys = System::new();

        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sys_pid]),
            false,
            ProcessRefreshKind::nothing(),
        );

        let process = sys.process(sys_pid).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Process with PID {} not found", pid),
            )
        })?;

        Ok(ProcessMetadata {
            pid,
            name: process.name().to_string_lossy().to_string(),
            start_time: process.start_time(),
        })
    }
}
