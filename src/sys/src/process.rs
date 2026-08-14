use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::io::ErrorKind;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::path::Path;
use tokio::process::Command;
use tokio::time::Duration;
use tracing::{info, instrument, trace, warn};

use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use crate::traits::ProcessControl;

#[cfg(windows)]
use std::os::windows::io::OwnedHandle;
#[cfg(target_os = "linux")]
use tokio::io::unix::AsyncFd;

#[derive(Debug)]
pub struct ProcessHandle<T> {
    pub(crate) process_ref: T,
    pub(crate) metadata: ProcessMetadata,
}

#[derive(Debug)]
pub struct Process {
    #[cfg(target_os = "linux")]
    pub(crate) handle: ProcessHandle<AsyncFd<OwnedFd>>,
    #[cfg(windows)]
    pub(crate) handle: ProcessHandle<OwnedHandle>,
    #[cfg(target_os = "macos")]
    pub(crate) handle: ProcessHandle<ProcessMetadata>,
}

impl Process {
    pub fn pid(&self) -> u32 {
        self.handle.metadata.pid
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

        let handle = ProcessHandle::bind(metadata.clone()).map_err(ProcessError::Io)?;
        let metadata_after = Self::spawn_extract_metadata(pid).await?;

        if metadata == metadata_after {
            Ok(Self { handle })
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

    pub async fn kill(self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.kill()?;
        self.handle.wait(timeout).await
    }

    pub async fn terminate(self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.terminate()?;
        self.handle.wait(timeout).await
    }

    pub async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError> {
        self.handle.check_permissions()?;
        self.handle.wait(timeout).await
    }
}
