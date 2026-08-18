use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND, STILL_ACTIVE};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};

pub fn map_signal_error(error: io::Error) -> ProcessError {
    match error.raw_os_error() {
        Some(code) if code == ERROR_INVALID_PARAMETER as i32 || code == ERROR_NOT_FOUND as i32 => {
            ProcessError::NotRunning
        }
        _ => ProcessError::Io(error),
    }
}

#[derive(Debug)]
pub struct WindowsProcessHandle {
    pub(crate) handle: OwnedHandle,
}

impl WindowsProcessHandle {
    fn is_process_exited(&self) -> io::Result<bool> {
        let mut exit_code = 0u32;

        let result = unsafe {
            GetExitCodeProcess(self.handle.as_raw_handle(), &mut exit_code)
        };

        if result == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(exit_code != STILL_ACTIVE as u32)
    }
}

#[async_trait::async_trait]
impl ProcessControl for WindowsProcessHandle {
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError> {
        let raw = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                metadata.pid,
            )
        };

        if raw.is_null() {
            let error = unsafe { GetLastError() };
            return Err(map_signal_error(io::Error::from_raw_os_error(error as i32)));
        }

        Ok(Self {
            handle: unsafe { OwnedHandle::from_raw_handle(raw.cast()) },
        })
    }

    fn kill(&self) -> Result<(), ProcessError> {
        let raw = self.handle.as_raw_handle();

        let result = unsafe {
            TerminateProcess(raw, 1)
        };
        if result != 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();

        if self.is_process_exited().map_err(ProcessError::Io)? {
            return Ok(());
        }

        Err(ProcessError::Io(error))
    }

    fn terminate(&self) -> Result<(), ProcessError> {
        self.kill()
    }

    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        let handle = self.handle.try_clone()?;

        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;

        tokio::task::spawn_blocking(move || {
            let result = unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    handle.as_raw_handle(),
                    timeout_ms,
                )
            };

            match result {
                windows_sys::Win32::Foundation::WAIT_OBJECT_0 => Ok(true),
                windows_sys::Win32::Foundation::WAIT_TIMEOUT => Ok(false),
                windows_sys::Win32::Foundation::WAIT_FAILED => {
                    Err(io::Error::last_os_error().into())
                }

                _ => Err(io::Error::other("unexpected WaitForSingleObject result").into()),
            }
        })
        .await
        .map_err(io::Error::other)?
    }

    fn check_permissions(&self) -> Result<(), ProcessError> {
        Ok(())
    }
}
