use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::traits::ProcessControl;
use std::io;
use std::time::Duration;

pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(libc::ESRCH) => ProcessError::NotRunning,
        _ => ProcessError::Io(error),
    }
}

#[derive(Debug)]
pub struct MacosProcessHandle {
    pub(crate) fingerprint: ProcessMetadata,
}

impl MacosProcessHandle {
    fn ensure_pid_not_reused(&self) -> Result<(), ProcessError> {
        match ProcessMetadata::extract(self.fingerprint.pid) {
            Ok(metadata) if metadata == self.fingerprint => Ok(()),

            Ok(_) => Err(ProcessError::Reused),

            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(ProcessError::NotRunning),

            Err(error) => Err(ProcessError::Io(error)),
        }
    }

    fn check_identity(&self) -> Result<bool, ProcessError> {
        match ProcessMetadata::extract(self.fingerprint.pid) {
            Ok(metadata) if metadata == self.fingerprint => Ok(true),
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

#[async_trait::async_trait]
impl ProcessControl for MacosProcessHandle {
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
        Ok(Self {
            fingerprint: metadata.clone(),
        })
    }

    fn kill(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;

        send_signal(self.fingerprint.pid, libc::SIGKILL).map_err(map_signal_error)?;

        Ok(())
    }

    fn terminate(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;

        send_signal(self.fingerprint.pid, libc::SIGTERM).map_err(map_signal_error)?;

        Ok(())
    }

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

    fn check_permissions(&self) -> Result<(), ProcessError> {
        self.ensure_pid_not_reused()?;
        send_signal(self.fingerprint.pid, 0).map_err(map_signal_error)?;
        Ok(())
    }
}
