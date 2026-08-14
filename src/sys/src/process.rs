use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::Path;
use tokio::process::Command;
use tokio::time::Duration;
use tracing::{info, instrument, trace, warn};

use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::traits::ProcessControl;
use crate::sys;

#[cfg(target_os = "linux")]
pub type Process = PlatformProcess<sys::linux::LinuxProcessHandle>;
#[cfg(windows)]
pub type Process = PlatformProcess<sys::windows::WindowsProcessHandle>;
#[cfg(target_os = "macos")]
pub type Process = PlatformProcess<sys::macos::MacosProcessHandle>;

#[derive(Debug)]
pub struct PlatformProcess<T: ProcessControl> {
    pub(crate) handle: T,
    pub(crate) metadata: ProcessMetadata,
}

impl<T: ProcessControl> Deref for PlatformProcess<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl<T: ProcessControl> PlatformProcess<T> {
    pub fn pid(&self) -> u32 {
        self.metadata.pid
    }

    async fn compare(expected_name: &str, actual_name: &str) -> Result<(), ProcessError> {
        if crate::utils::process_names_match(actual_name, expected_name) {
            Ok(())
        } else {
            Err(ProcessError::Mismatch {
                expected: expected_name.to_string(),
                actual: actual_name.to_string(),
            })
        }
    }

    async fn spawn_extract_metadata(pid: u32) -> Result<ProcessMetadata, ProcessError> {
        let metadata = tokio::task::spawn_blocking(move || ProcessMetadata::extract(pid))
            .await
            .map_err(|e| ProcessError::Io(io::Error::other(e)))?
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => ProcessError::NotRunning,
                _ => ProcessError::Io(e),
            })?;
        Ok(metadata)
    }

    #[instrument(
        skip_all,
        name = "process_bind",
        fields(
            expected = %expected_name,
            pid = pid,
        )
    )]
    pub async fn bind(pid: u32, expected_name: String) -> Result<Self, ProcessError> {
        let metadata = Self::spawn_extract_metadata(pid).await?;
        Self::compare(&expected_name, &metadata.name).await?;

        let handle = T::bind(metadata.clone()).map_err(ProcessError::Io)?;
        let metadata_after = Self::spawn_extract_metadata(pid).await?;

        if metadata == metadata_after {
            Ok(Self { handle, metadata })
        } else {
            Err(ProcessError::Mismatch {
                expected: expected_name,
                actual: metadata_after.name,
            })
        }
    }

    pub async fn spawn(binary: &Path) -> Result<Self, ProcessError> {
        let args: [String; 0] = [];
        Self::spawn_with_args(binary, &args).await
    }

    #[instrument(
        skip_all,
        name = "process_spawn",
        fields(
            bin = %binary.display(),
        ))]
    pub async fn spawn_with_args(
        binary: &Path,
        args: &[impl AsRef<OsStr>],
    ) -> Result<Self, ProcessError> {
        trace!("spawning process...");
        let child = Command::new(binary)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let binary_name = binary.file_name().ok_or_else(|| {
            io::Error::other(format!(
                "failed to get file name of binary: {}",
                binary.to_string_lossy()
            ))
        })?;
        match child.id() {
            Some(id) => {
                info!("process successfully spawned with PID: {}", id);
                Ok(Self::bind(id, binary_name.to_string_lossy().to_string()).await?)
            }
            None => {
                warn!("daemon process exited immediately after spawning.");
                Err(io::Error::other(format!(
                    "daemon process at '{}' exited immediately and yielded no PID. It might have crashed on startup.",
                    binary.to_string_lossy()
                )).into())
            }
        }
    }

    pub async fn kill(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.kill()?;
        self.handle.wait(timeout).await
    }

    pub async fn terminate(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.terminate()?;
        self.handle.wait(timeout).await
    }

    pub async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.wait(timeout).await
    }
}
