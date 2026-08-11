use std::io;
use std::path::Path;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use thiserror::Error;
use tokio::process::Command;
use tracing::{error, info, instrument, trace, warn};

pub trait ProcessControl {
    fn kill(&self) -> std::io::Result<()>;
    fn pid(&self) -> u32;
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process is not running")]
    NotRunning,
    #[error("process name mismatch: expected {0}")]
    Mismatch(String),
}

#[derive(Debug)]
pub struct Process {
    pub(crate) pid: u32,
}

impl Process {
    fn new(pid: u32) -> Self {
        Self { pid }
    }

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
                    if actual_name.eq_ignore_ascii_case(&expected_name) {
                        Ok(Self::new(pid))
                    } else {
                        Err(ProcessError::Mismatch(actual_name.to_string()))
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
        name = "process_spawn",
        fields(
            bin = %binary.display(),
        ))]
    pub fn spawn(binary: &Path) -> io::Result<Self> {
        let mut command = Command::new(binary);
        trace!("spawning process...");

        let child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e: io::Error| {
                error!(error = %e, "failed to spawn process");
                e
            })?;

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
