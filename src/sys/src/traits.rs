use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use std::fmt::Debug;
use std::io;
use tokio::time::Duration;

#[async_trait::async_trait]
pub trait ProcessControl: Debug + Sized {
    fn bind(metadata: ProcessMetadata) -> io::Result<Self>;
    fn kill(&self) -> Result<bool, ProcessError>;
    fn terminate(&self) -> Result<bool, ProcessError>;
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError>;
    fn check_permissions(&self) -> Result<bool, ProcessError>;
}
