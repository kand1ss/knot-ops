use crate::process::ProcessHandle;
use crate::traits::ProcessControl;

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess,
};

#[async_trait::async_trait]
impl ProcessControl for ProcessHandle<OwnedHandle> {
    fn open_process(pid: u32) -> io::Result<Self> {
        let raw = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };

        if raw.is_null() {
            let error = unsafe { GetLastError() };
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        Ok(Self {
            pid,
            process_ref: unsafe { OwnedHandle::from_raw_handle(raw.cast()) },
        })
    }

    fn executable_name(&self) -> io::Result<String> {
        let raw = self.process_ref.as_raw_handle();
        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;

        let ok = unsafe { QueryFullProcessImageNameW(raw, 0, buf.as_mut_ptr(), &mut size) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }

        let path = String::from_utf16_lossy(&buf[..size as usize]);
        let actual_name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        Ok(actual_name)
    }

    fn kill(&self) -> io::Result<()> {
        let raw = self.process_ref.as_raw_handle();
        let ok = unsafe { TerminateProcess(raw, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn terminate(&self) -> io::Result<()> {
        self.kill()
    }

    async fn wait(&self, timeout: Duration) -> io::Result<bool> {
        let handle = self.process_ref.as_raw_handle();

        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;

        tokio::task::spawn_blocking(move || {
            let result = unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(handle, timeout_ms)
            };

            match result {
                windows_sys::Win32::Foundation::WAIT_OBJECT_0 => Ok(true),
                windows_sys::Win32::Foundation::WAIT_TIMEOUT => Ok(false),
                windows_sys::Win32::Foundation::WAIT_FAILED => Err(io::Error::last_os_error()),

                _ => Err(io::Error::new(
                    io::ErrorKind::Other,
                    "unexpected WaitForSingleObject result",
                )),
            }
        })
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
    }

    fn check_permissions(&self) -> io::Result<()> {
        Ok(())
    }
}
