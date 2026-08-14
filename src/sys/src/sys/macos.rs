use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::process::ProcessHandle;
use crate::traits::ProcessControl;
use std::io;
use std::time::Duration;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System};

impl ProcessHandle<ProcessMetadata> {
    fn ensure_pid_not_reused(&self) -> io::Result<bool> {
        let metadata = ProcessMetadata::extract(self.process_ref.pid)?;
        Ok(metadata == self.process_ref)
    }

    fn check_identity(&self) -> Result<bool, ProcessError> {
        match ProcessMetadata::extract(self.process_ref.pid) {
            Ok(metadata) if metadata == self.process_ref => Ok(true),
            Ok(_) => Err(ProcessError::Reused),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(ProcessError::Io(e)),
        }
    }
}

fn send_signal(pid: u32, signal: libc::c_int) -> io::Result<()> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };

    if result == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

impl ProcessControl for ProcessHandle<ProcessMetadata> {
    fn bind(metadata: ProcessMetadata) -> io::Result<Self> {
        Ok(Self {
            process_ref: metadata.clone(),
            metadata,
        })
    }

    fn kill(&self) -> Result<bool, ProcessError> {
        match self.ensure_pid_not_reused()? {
            true => send_signal(self.process_ref.pid, libc::SIGKILL).map_err(ProcessError::Io)?,
            false => return Err(ProcessError::Reused),
        }
        Ok(true)
    }

    fn terminate(&self) -> Result<bool, ProcessError> {
        match self.ensure_pid_not_reused()? {
            true => send_signal(self.process_ref.pid, libc::SIGTERM).map_err(ProcessError::Io)?,
            false => return Err(ProcessError::Reused),
        }
        Ok(true)
    }

    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let handle = self.process_ref.clone();
            let exists = tokio::task::spawn_blocking(move || {
                let owned = ProcessHandle {
                    process_ref: handle.clone(),
                    metadata: handle,
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

    fn check_permissions(&self) -> Result<bool, ProcessError> {
        match self.ensure_pid_not_reused()? {
            true => send_signal(self.process_ref.pid, 0).map_err(ProcessError::Io)?,
            false => return Err(ProcessError::Reused),
        }
        Ok(true)
    }
}
