use crate::process::ProcessHandle;
use crate::traits::ProcessControl;

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;
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

#[async_trait::async_trait]
impl ProcessControl for ProcessHandle<ProcessRef> {
    fn open_process(pid: u32) -> io::Result<Self> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let fd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
        Ok(Self {
            pid,
            process_ref: ProcessRef::new(fd)?,
        })
    }

    fn executable_name(&self) -> io::Result<String> {
        let pid = self.pid;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))?;
        Ok(comm)
    }

    fn kill(&self) -> io::Result<()> {
        send_signal(&self.process_ref, libc::SIGKILL)
    }

    fn terminate(&self) -> io::Result<()> {
        send_signal(&self.process_ref, libc::SIGTERM)
    }

    async fn wait(&self, timeout: Duration) -> io::Result<bool> {
        if timeout.is_zero() {
            let mut guard = self.process_ref.readable().await?;
            return match guard.try_io(|_| Ok(())) {
                Ok(result) => {
                    result?;
                    Ok(true)
                }
                Err(_) => Ok(false),
            };
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

    fn check_permissions(&self) -> io::Result<()> {
        send_signal(&self.process_ref, 0)
    }
}
