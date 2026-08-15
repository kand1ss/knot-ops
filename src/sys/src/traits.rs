use crate::errors::ProcessError;
use crate::metadata::ProcessMetadata;
use std::fmt::Debug;
use tokio::time::Duration;

#[async_trait::async_trait]
pub trait ProcessControl: Debug + Sized {
    fn bind(metadata: ProcessMetadata) -> Result<Self, ProcessError>;
    fn kill(&self) -> Result<(), ProcessError>;
    fn terminate(&self) -> Result<(), ProcessError>;
    async fn wait(&self, timeout: Duration) -> Result<bool, ProcessError>;
    fn check_permissions(&self) -> Result<(), ProcessError>;
}
