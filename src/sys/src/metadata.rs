use std::io;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ProcessMetadata {
    pub pid: u32,
    pub name: String,
    pub start_time: u64,
}

impl ProcessMetadata {
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
