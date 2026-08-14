use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

type ProcessRef = AsyncFd<OwnedFd>;

pub fn send_signal(pid_fd: &AsyncFd<OwnedFd>, signal: libc::c_int) -> io::Result<()> {
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
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[derive(Debug)]
pub struct LinuxProcessHandle {
    pub(crate) process_ref: ProcessRef
}

#[async_trait::async_trait]
impl ProcessControl for LinuxProcessHandle {
    fn bind(metadata: ProcessMetadata) -> io::Result<Self> {
        let pid = metadata.pid;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let fd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
        Ok(Self {
            process_ref: ProcessRef::new(fd)?,
        })
    }

    fn kill(&self) -> Result<bool, ProcessError> {
        send_signal(&self.process_ref, libc::SIGKILL)?;
        Ok(true)
    }

    fn terminate(&self) -> Result<bool, ProcessError> {
        send_signal(&self.process_ref, libc::SIGTERM)?;
        Ok(true)
    }

    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        if timeout.is_zero() {
            match self.process_ref.try_io(Interest::READABLE, |_| Ok(())) {
                Ok(_) => {
                    return Ok(true);
                }
                Err(_) => return Ok(false),
            }
        }

        let wait = async {
            loop {
                let mut guard = self.process_ref.readable().await?;
                match guard.try_io(|_| Ok(())) {
                    Ok(result) => {
                        result?;
                        return Ok(true);
                    }
                    Err(_) => continue,
                }
            }
        };

        tokio::time::timeout(timeout, wait)
            .await
            .unwrap_or(Ok(false))
    }

    fn check_permissions(&self) -> Result<bool, ProcessError> {
        send_signal(&self.process_ref, 0)?;
        Ok(true)
    }
}
