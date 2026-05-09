use crate::{
    launcher::{DaemonLauncher, DefaultLauncher},
    stream::InboxStream,
    utils::recursively_find_knot,
};
use knot_core::{
    consts::{KNOT_PID_FILE, KNOT_SOCKET_FILE},
    errors::{
        ClientError, DaemonLifecycleError, HealthcheckError, ProtocolError, TransportError,
        WorkspaceError,
    },
};
use knot_protocol::daemon::{
    DaemonRequest, DaemonResponse, DaemonTransportSpec, ServiceStatusResponse,
};
use knot_transport::transport::{MessageTransport, RawTransport, ipc::IpcTransport};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tracing::{debug, error, info, instrument, warn};

/// A client for interacting with the `knot` daemon.
///
/// `KnotClient` provides methods to manage the daemon lifecycle, perform health checks,
/// and execute commands like `up`, `down`, and `status`. It uses an underlying
/// `RawTransport` (typically `IpcTransport`) to communicate with the daemon.
pub struct KnotClient<R: RawTransport> {
    transport: Option<Arc<MessageTransport<R, DaemonTransportSpec>>>,
    directory: PathBuf,
    default_timeout: Duration,
    default_retries: u8,
    daemon_launcher: Box<dyn DaemonLauncher>,
}

impl KnotClient<IpcTransport> {
    /// Connects to the daemon in the specified directory.
    ///
    /// This method searches for the `.knot` folder, locates the IPC socket,
    /// and establishes a connection.
    #[instrument(skip_all, fields(directory = %directory.display()))]
    pub async fn connect_to_directory(directory: &Path) -> Result<Self, ClientError> {
        let dir = directory.to_path_buf();
        let folder_path = recursively_find_knot(&dir).ok_or(WorkspaceError::NotInitialized(
            dir.to_string_lossy().into_owned(),
        ))?;
        let socket_path = folder_path.join(KNOT_SOCKET_FILE);

        let transport = Self::establish_transport(socket_path).await;
        Ok(Self::new(folder_path, transport))
    }

    async fn establish_transport(
        socket_path: PathBuf,
    ) -> Option<MessageTransport<IpcTransport, DaemonTransportSpec>> {
        #[cfg(windows)]
        {
            IpcTransport::connect(socket_path)
                .await
                .ok()
                .map(|t| t.to_messaged())
        }

        #[cfg(not(windows))]
        {
            if socket_path.exists() {
                IpcTransport::connect(socket_path)
                    .await
                    .ok()
                    .map(|t| t.to_messaged())
            } else {
                None
            }
        }
    }

    async fn is_health(&self) -> bool {
        self.is_connected() && self.healthcheck().await.is_ok()
    }

    /// Connects to the daemon or launches it if it's not running or unhealthy.
    #[instrument(skip_all, fields(directory = %directory.display()))]
    pub async fn connect_or_launch(directory: &Path) -> Result<Self, ClientError> {
        let client = Self::connect_to_directory(directory).await?;
        if client.is_health().await {
            Ok(client)
        } else {
            info!("Daemon is not running or unhealthy, launching...");
            client.launch_daemon().await
        }
    }

    /// Launches the daemon process and waits for it to become healthy.
    #[instrument(skip(self), name = "launch_daemon")]
    pub async fn launch_daemon(self) -> Result<Self, ClientError> {
        info!("Spawning daemon process...");
        let _pid = self.daemon_launcher.launch(&self.directory).await?;

        let mut retries = 10;
        let delay = Duration::from_millis(200);

        loop {
            tokio::time::sleep(delay).await;

            debug!(
                "Attempting to connect to launched daemon (retries left: {})...",
                retries
            );
            let client = Self::connect_to_directory(&self.directory).await?;
            if client.is_health().await {
                info!("Daemon launched and healthy.");
                return Ok(client);
            }

            retries -= 1;
            if retries == 0 {
                error!("Daemon launch timeout: socket never appeared.");
                return Err(DaemonLifecycleError::LaunchFailed {
                    message: "Daemon process was spawned, but IPC socket never appeared"
                        .to_string(),
                    binary_path: self
                        .daemon_launcher
                        .binary_path()
                        .to_string_lossy()
                        .into_owned(),
                    target_dir: self.directory.to_string_lossy().into_owned(),
                    error: "Socket not found".to_string(),
                }
                .into());
            }
        }
    }

    /// Performs a health check on the daemon.
    ///
    /// This checks if the socket exists, if the PID file is valid, and if the
    /// daemon responds to a `ping` request.
    #[instrument(skip(self), name = "healthcheck")]
    pub async fn healthcheck(&self) -> Result<(), ClientError> {
        if self.transport.is_none() {
            return Err(HealthcheckError::NotConnected.into());
        }

        self.check_socket()?;
        self.read_and_check_pid().await?;
        if let Err(e) = self.ping().await {
            if matches!(e, ClientError::Transport(TransportError::Timeout { .. })) {
                warn!("Daemon ping timeout during healthcheck.");
                return Err(HealthcheckError::DaemonNotResponding.into());
            }
            return Err(e);
        }
        Ok(())
    }

    fn check_socket(&self) -> Result<(), HealthcheckError> {
        #[cfg(windows)]
        {
            if !self.pid_path().exists() {
                return Err(HealthcheckError::InconsistentState(
                    "Transport exists, but file which contains daemon pid are not exists"
                        .to_string(),
                ));
            }
        }
        #[cfg(not(windows))]
        {
            if self.socket_path().exists() {
                if !self.pid_path().exists() {
                    return Err(HealthcheckError::StaleSocket(
                        self.socket_path().to_path_buf(),
                    ));
                }
            } else {
                return Err(HealthcheckError::InconsistentState(
                    "Transport exists, but socket file are not exists".to_string(),
                ));
            }
        }

        Ok(())
    }

    async fn read_and_check_pid(&self) -> Result<(), ClientError> {
        let path = self.pid_path();

        let pid_val = Self::read_pid_as_usize(&path)
            .await
            .ok_or_else(|| WorkspaceError::BrokenData(path.clone()))?;

        let mut sys = System::new();
        let pid = Pid::from(pid_val);

        let _process_exists = sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing(),
        );

        if let Some(process) = sys.process(pid) {
            match process.status() {
                sysinfo::ProcessStatus::Zombie => {
                    Err(HealthcheckError::ZombieProcess(pid.as_u32()).into())
                }
                _ => Ok(()),
            }
        } else {
            Err(HealthcheckError::ProcessNotExists(pid.as_u32()).into())
        }
    }

    async fn read_pid_as_usize(path: &Path) -> Option<usize> {
        let content = tokio::fs::read_to_string(path).await.ok()?;
        content.trim().parse::<usize>().ok()
    }

    /// Attempts to repair the daemon environment based on the identified issue.
    ///
    /// This may involve cleaning up stale socket/PID files or force-killing
    /// unresponsive daemon processes.
    #[instrument(skip(self), name = "repair")]
    pub async fn repair(&self, issue: &HealthcheckError) -> Result<(), ClientError> {
        warn!("Attempting to repair daemon environment: {}", issue);

        match issue {
            HealthcheckError::StaleSocket(_)
            | HealthcheckError::ProcessNotExists(_)
            | HealthcheckError::InconsistentState(_) => {
                self.clean_volatile_files().await;
            }

            HealthcheckError::DaemonNotResponding => {
                if let Some(pid) = Self::read_pid_as_usize(&self.pid_path()).await {
                    self.force_kill_process(pid).await;
                } else {
                    self.clean_volatile_files().await;
                }
            }

            HealthcheckError::ZombieProcess(pid) => {
                self.force_kill_process(*pid as usize).await;
            }

            HealthcheckError::NotConnected => {}
        }

        Ok(())
    }

    async fn clean_volatile_files(&self) {
        #[cfg(not(windows))]
        tokio::fs::remove_file(self.socket_path()).await.ok();

        tokio::fs::remove_file(self.pid_path()).await.ok();
        tracing::debug!("Volatile files (.sock, .pid) cleaned up.");
    }

    async fn force_kill_process(&self, pid: usize) {
        let mut sys = System::new();
        let sys_pid = Pid::from(pid);

        let _process_exists = sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sys_pid]),
            false,
            ProcessRefreshKind::nothing(),
        );

        if let Some(process) = sys.process(sys_pid) {
            tracing::warn!("Sending SIGKILL to daemon process {}", pid);
            process.kill_with(sysinfo::Signal::Kill);
        }

        self.clean_volatile_files().await;
    }
}

impl<R: RawTransport> KnotClient<R> {
    /// Creates a new `KnotClient` instance.
    pub fn new(
        directory: PathBuf,
        transport: Option<MessageTransport<R, DaemonTransportSpec>>,
    ) -> Self {
        let transport = transport.map(Arc::new);

        Self {
            directory,
            transport,
            default_timeout: Duration::from_secs(10),
            default_retries: 0,
            daemon_launcher: Box::new(DefaultLauncher::new()),
        }
    }

    #[cfg(not(windows))]
    fn socket_path(&self) -> PathBuf {
        self.directory.join(KNOT_SOCKET_FILE)
    }

    fn pid_path(&self) -> PathBuf {
        self.directory.join(KNOT_PID_FILE)
    }

    /// Sets the default timeout for daemon requests.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Sets the number of retries for daemon requests.
    pub fn with_retries(mut self, retries: u8) -> Self {
        self.default_retries = retries;
        self
    }

    /// Sets a custom daemon launcher.
    pub fn with_launcher(mut self, launcher: impl DaemonLauncher + 'static) -> Self {
        self.daemon_launcher = Box::new(launcher);
        self
    }

    /// Checks if the client is currently connected to the daemon.
    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    fn ensure_connected(&self) -> Result<(), ClientError> {
        if !self.is_connected() {
            return Err(
                DaemonLifecycleError::NotRunning(self.directory.join(KNOT_SOCKET_FILE)).into(),
            );
        }
        Ok(())
    }

    fn ensure_transport(
        &self,
    ) -> Result<Arc<MessageTransport<R, DaemonTransportSpec>>, ClientError> {
        self.ensure_connected()?;
        Ok(Arc::clone(self.transport.as_ref().unwrap()))
    }

    async fn execute_with_stream(
        &self,
        req: DaemonRequest,
    ) -> Result<InboxStream<R, DaemonTransportSpec>, ClientError> {
        let transport = self.ensure_transport()?;

        match self.execute_request(req).await? {
            DaemonResponse::Ok | DaemonResponse::Done => Ok(InboxStream::new(transport)),
            DaemonResponse::Error(msg) => Err(ProtocolError::CommandFailed(msg).into()),
            _ => Err(ProtocolError::UnexpectedResponse {
                expected: "DaemonResponse::Ok".to_string(),
            }
            .into()),
        }
    }

    async fn execute_request(&self, req: DaemonRequest) -> Result<DaemonResponse, ClientError> {
        let mut current_retries = self.default_retries;
        let retry_delay = Duration::from_millis(200);
        let transport = self.ensure_transport()?;

        loop {
            match transport
                .request(req.clone(), self.default_timeout.as_secs(), None)
                .await
            {
                Ok(response) => {
                    return Ok(response);
                }
                Err(e) => {
                    if current_retries == 0 {
                        return Err(ClientError::Transport(e));
                    }
                    current_retries -= 1;
                    tokio::time::sleep(retry_delay).await;
                }
            }
        }
    }

    /// Pings the daemon to check connectivity.
    #[instrument(skip(self), name = "ping")]
    pub async fn ping(&self) -> Result<(), ClientError> {
        match self.execute_request(DaemonRequest::Ping).await? {
            DaemonResponse::Pong => Ok(()),
            _ => Err(ProtocolError::UnexpectedResponse {
                expected: "DaemonResponse::Pong".to_string(),
            }
            .into()),
        }
    }

    /// Starts all services managed by the daemon.
    #[instrument(skip(self), name = "up")]
    pub async fn up(&self) -> Result<InboxStream<R, DaemonTransportSpec>, ClientError> {
        self.execute_with_stream(DaemonRequest::Up).await
    }

    /// Stops all services managed by the daemon.
    #[instrument(skip(self), name = "down")]
    pub async fn down(&self) -> Result<InboxStream<R, DaemonTransportSpec>, ClientError> {
        self.execute_with_stream(DaemonRequest::Down).await
    }

    /// Returns the status of all services managed by the daemon.
    #[instrument(skip(self), name = "status")]
    pub async fn status(&self) -> Result<Vec<ServiceStatusResponse>, ClientError> {
        match self.execute_request(DaemonRequest::Status).await? {
            DaemonResponse::Status(services) => Ok(services),
            _ => Err(ProtocolError::UnexpectedResponse {
                expected: "DaemonResponse::Status".to_string(),
            }
            .into()),
        }
    }
}
