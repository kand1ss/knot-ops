use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;
use tokio::io::unix::AsyncFd;

type ProcessRef = AsyncFd<OwnedFd>;

pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(libc::ESRCH) => ProcessError::NotRunning,
        _ => ProcessError::Io(error),
    }
}

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

#[derive(Debug)]
pub struct LinuxProcessHandle {
    pub(crate) process_ref: ProcessRef,
}

impl LinuxProcessHandle {
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

    fn kill(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, libc::SIGKILL)?;
        Ok(())
    }

    fn terminate(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, libc::SIGTERM)?;
        Ok(())
    }

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

    fn check_permissions(&self) -> Result<(), ProcessError> {
        send_signal(&self.process_ref, 0)?;
        Ok(())
    }
}
