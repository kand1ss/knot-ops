use std::fmt::Debug;
use std::io;
use tokio::time::Duration;

#[async_trait::async_trait]
pub trait ProcessControl: Debug + Sized {
    fn open_process(pid: u32) -> io::Result<Self>;
    fn executable_name(&self) -> io::Result<String>;
    fn kill(&self) -> io::Result<()>;
    fn terminate(&self) -> io::Result<()>;
    async fn wait(&self, timeout: Duration) -> io::Result<bool>;
    fn check_permissions(&self) -> io::Result<()>;
}
