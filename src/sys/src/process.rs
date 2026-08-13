use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::path::Path;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::process::Command;
use tokio::time::Duration;
use tracing::{info, instrument, trace, warn};

use crate::errors::ProcessError;
use crate::traits::ProcessControl;

#[cfg(windows)]
use std::os::windows::io::OwnedHandle;
#[cfg(target_os = "linux")]
use tokio::io::unix::AsyncFd;

#[derive(Debug)]
pub struct ProcessHandle<T> {
    pub(crate) process_ref: T,
    pub(crate) pid: u32,
}

#[derive(Debug)]
pub struct Process {
    #[cfg(target_os = "linux")]
    pub(crate) handle: ProcessHandle<AsyncFd<OwnedFd>>,
    #[cfg(windows)]
    pub(crate) handle: ProcessHandle<OwnedHandle>,
}

impl Process {
    pub fn pid(&self) -> u32 {
        self.handle.pid
    }

    async fn acquire(pid: u32, expected_name: String) -> Result<(), ProcessError> {
        let sys_pid = Pid::from(pid as usize);
        let expected_name = expected_name.clone();
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
                    if crate::utils::process_names_match(&actual_name, &expected_name) {
                        Ok(())
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

    #[instrument(
        skip_all,
        name = "process_bind",
        fields(
            expected = %expected_name,
            pid = pid,
        )
    )]
    pub async fn bind(pid: u32, expected_name: String) -> Result<Self, ProcessError> {
        Self::acquire(pid, expected_name.clone()).await?;

        let handle = ProcessHandle::open_process(pid).map_err(ProcessError::Io)?;
        let process_name = handle.executable_name().map_err(ProcessError::Io)?;

        if crate::utils::process_names_match(&process_name, &expected_name) {
            info!("successfully bound to process");
        } else {
            warn!(actual = %process_name, "process name mismatch; cannot bind to process");
            return Err(ProcessError::Mismatch {
                expected: expected_name,
                actual: process_name,
            });
        }

        Ok(Self { handle })
    }

    pub async fn spawn(binary: &Path) -> Result<Self, ProcessError> {
        let args: [String; 0] = [];
        Self::spawn_with_args(binary, &args).await
    }

    #[instrument(
        skip_all,
        name = "process_spawn",
        fields(
            bin = %binary.display(),
        ))]
    pub async fn spawn_with_args(
        binary: &Path,
        args: &[impl AsRef<OsStr>],
    ) -> Result<Self, ProcessError> {
        trace!("spawning process...");
        let child = Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let binary_name = binary.file_name().ok_or_else(|| {
            io::Error::other(format!(
                "failed to get file name of binary: {}",
                binary.to_string_lossy()
            ))
        })?;
        match child.id() {
            Some(id) => {
                info!("process successfully spawned with PID: {}", id);
                Ok(Self::bind(id, binary_name.to_string_lossy().to_string()).await?)
            }
            None => {
                warn!("daemon process exited immediately after spawning.");
                Err(io::Error::other(format!(
                    "daemon process at '{}' exited immediately and yielded no PID. It might have crashed on startup.",
                    binary.to_string_lossy()
                )).into())
            }
        }
    }

    pub async fn kill(self, timeout: Duration) -> io::Result<bool> {
        self.handle.check_permissions()?;
        self.handle.kill()?;
        self.handle.wait(timeout).await
    }

    pub async fn terminate(self, timeout: Duration) -> io::Result<bool> {
        self.handle.check_permissions()?;
        self.handle.terminate()?;
        self.handle.wait(timeout).await
    }
}
