use std::io::Result;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub trait ProcessControl {
    fn kill(&self) -> Result<()>;
    fn pid(&self) -> u32;
}

pub enum ProcessVerification {
    NotRunning,
    Valid,
    Mismatch(String),
}

pub struct Process {
    pub(crate) pid: u32,
    pub(crate) name: String,
}

impl Process {
    pub fn new(pid: u32, name: String) -> Self {
        Self { pid, name }
    }

    pub fn verify(&self) -> ProcessVerification {
        let pid = Pid::from(self.pid as usize);
        let mut sys = System::new();

        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing(),
        );

        match sys.process(pid) {
            Some(process) => {
                let actual_name = process.name().to_string_lossy();
                if actual_name.eq_ignore_ascii_case(&self.name) {
                    ProcessVerification::Valid
                } else {
                    ProcessVerification::Mismatch(actual_name.to_string())
                }
            }
            None => ProcessVerification::NotRunning,
        }
    }
}

impl ProcessControl for Process {
    #[cfg(windows)]
    fn kill(&self) -> Result<()> {
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
    fn kill(&self) -> Result<()> {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        kill(Pid::from_raw(self.pid as i32), Signal::SIGKILL)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}
