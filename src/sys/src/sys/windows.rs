use crate::process::ProcessHandle;
use crate::traits::ProcessControl;

use crate::ProcessError;
use crate::metadata::ProcessMetadata;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    TerminateProcess,
};

#[async_trait::async_trait]
impl ProcessControl for ProcessHandle<OwnedHandle> {
    fn bind(metadata: ProcessMetadata) -> io::Result<Self> {
        let raw = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                metadata.pid,
            )
        };

        if raw.is_null() {
            let error = unsafe { GetLastError() };
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        Ok(Self {
            metadata,
            process_ref: unsafe { OwnedHandle::from_raw_handle(raw.cast()) },
        })
    }

    fn kill(&self) -> Result<bool, ProcessError> {
        let raw = self.process_ref.as_raw_handle();
        let ok = unsafe { TerminateProcess(raw, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(true)
    }

    fn terminate(&self) -> Result<bool, ProcessError> {
        self.kill()
    }

    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        let handle = self.process_ref.try_clone()?;

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

    fn check_permissions(&self) -> Result<bool, ProcessError> {
        Ok(true)
    }
}
