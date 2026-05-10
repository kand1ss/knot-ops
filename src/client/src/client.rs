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
    DaemonEvent, DaemonRequest, DaemonResponse, DaemonTransportSpec, ServiceStatusResponse,
};
use knot_transport::{
    messages::Message,
    transport::{MessageTransport, RawTransport, ipc::IpcTransport},
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::sync::broadcast;
use tracing::{debug, error, info, instrument, warn};

/// The primary client for interacting with the `knot` background daemon.
///
/// `KnotClient` provides a high-level API to manage the daemon's lifecycle (launch, repair),
/// monitor its health, and execute workspace commands such as `up`, `down`, and `status`.
///
/// Under the hood, it multiplexes a single [`RawTransport`] (typically an IPC socket)
/// to handle both synchronous RPC requests (Command/Response) and asynchronous
/// event streaming via a background router task.
///
/// # Drop Behavior
///
/// When the `KnotClient` goes out of scope and is dropped, it will automatically
/// abort its internal background reader loop, preventing memory leaks and orphaned tasks.
pub struct KnotClient<R: RawTransport> {
    transport: Option<Arc<MessageTransport<R, DaemonTransportSpec>>>,
    directory: PathBuf,
    sender: broadcast::Sender<Message<DaemonRequest, DaemonResponse, DaemonEvent>>,
    loop_handle: Option<tokio::task::JoinHandle<()>>,
    default_timeout: Duration,
    default_retries: u8,
    daemon_launcher: Box<dyn DaemonLauncher + Send + Sync>,
}
impl<R: RawTransport> Drop for KnotClient<R> {
    fn drop(&mut self) {
        if let Some(h) = &self.loop_handle {
            h.abort();
        }
    }
}

impl KnotClient<IpcTransport> {
    /// Resolves the workspace and establishes a connection to the daemon.
    ///
    /// This method recursively searches upwards from the specified `directory` to locate
    /// the root `.knot` workspace folder. Once found, it attempts to connect to the
    /// IPC socket located within that folder.
    ///
    /// # Returns
    ///
    /// Returns a new `KnotClient` instance. Note that the connection may not be
    /// active or healthy yet. Use [`Self::healthcheck`] to verify the daemon's state.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] (specifically [`WorkspaceError::NotInitialized`]) if
    /// no `.knot` directory is found in the target path or its parents.
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

    /// Ensures a healthy connection to the daemon, launching it if necessary.
    ///
    /// This is a high-level, idempotent convenience method. It first attempts to connect
    /// to an existing daemon in the given directory. If the daemon is not running,
    /// or if it fails the health check, it will automatically spawn a new instance
    /// and establish a fresh connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] if the daemon fails to launch or if the connection
    /// cannot be established after spawning.    #[instrument(skip_all, fields(directory = %directory.display()))]
    pub async fn connect_or_launch(directory: &Path) -> Result<Self, ClientError> {
        let client = Self::connect_to_directory(directory).await?;
        if client.is_health().await {
            Ok(client)
        } else {
            info!("Daemon is not running or unhealthy, launching...");
            client.launch_daemon().await
        }
    }

    /// Spawns the daemon process and waits for it to become ready.
    ///
    /// This method uses the configured [`DaemonLauncher`] to start the background process.
    /// After spawning, it aggressively polls the socket directory until the daemon
    /// creates the IPC socket and passes a full health check.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] (specifically a [`DaemonLifecycleError::LaunchFailed`])
    /// if the daemon process fails to start, or if the socket does not appear within
    /// the retry limit.    #[instrument(skip(self), name = "launch_daemon")]
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

            if let Ok(client) = Self::connect_to_directory(&self.directory).await
                && client.is_health().await
            {
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

    /// Performs a comprehensive health check on the daemon instance.
    ///
    /// This method verifies the integrity of the connection in three stages:
    /// 1. Checks if the IPC socket file exists and matches the expected state.
    /// 2. Reads the PID file and ensures the corresponding process is actively running (not a zombie).
    /// 3. Sends a `ping` request to confirm the daemon is responsive.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] containing a [`HealthcheckError`] detailing
    /// exactly which stage of the check failed.
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

    /// Attempts to repair the daemon environment based on a specific health check failure.
    ///
    /// Depending on the `issue` provided, this method will take corrective actions
    /// such as deleting stale `.sock` and `.pid` files, or forcibly terminating
    /// unresponsive or zombie daemon processes.
    ///
    /// # Arguments
    ///
    /// * `issue` - The specific [`HealthcheckError`] that needs to be resolved.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` once the cleanup or termination tasks have been dispatched.    
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
        let (sender, _) = broadcast::channel(1024);
        let loop_handle = if let Some(t) = &transport {
            Some(tokio::spawn(Self::read_loop(sender.clone(), Arc::clone(t))))
        } else {
            None
        };

        Self {
            directory,
            transport,
            sender,
            loop_handle,
            default_timeout: Duration::from_secs(10),
            default_retries: 0,
            daemon_launcher: Box::new(DefaultLauncher::new()),
        }
    }

    async fn read_loop(
        sender: broadcast::Sender<Message<DaemonRequest, DaemonResponse, DaemonEvent>>,
        transport: Arc<MessageTransport<R, DaemonTransportSpec>>,
    ) {
        loop {
            match transport.recv().await {
                Ok(ctx) => {
                    let (message, _) = ctx.into_parts();
                    let _ = sender.send(message);
                }
                Err(e) => {
                    if e.is_fatal() {
                        break;
                    }
                }
            }
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
    pub fn with_launcher(mut self, launcher: impl DaemonLauncher + Send + Sync + 'static) -> Self {
        self.daemon_launcher = Box::new(launcher);
        self
    }

    /// Checks if the client is currently connected to the daemon.
    pub fn is_connected(&self) -> bool {
        if let Some(transport) = &self.transport {
            return transport.is_alive();
        }
        false
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
    ) -> Result<InboxStream<DaemonTransportSpec>, ClientError> {
        let receiver = self.sender.subscribe();

        match self.execute_request(req).await? {
            DaemonResponse::Ok | DaemonResponse::Done => Ok(InboxStream::new(receiver)),
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

    /// Returns a stream of events emitted by the daemon.
    ///
    /// This method creates a new subscription to the internal broadcast channel.
    /// Since it uses a broadcast mechanism, multiple subscribers can receive the
    /// same stream of events simultaneously.
    ///
    /// The returned [`InboxStream`] will receive all [`DaemonEvent`]s sent by the
    /// daemon after this method is called. Events sent prior to calling this
    /// method are not replayed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use tokio_stream::StreamExt;
    ///
    /// async fn example(client: KnotClient<IpcTransport>) -> Result<(), Box<dyn std::error::Error>> {
    ///     let mut events = client.stream();
    ///
    ///     while let Some(Ok(event)) = events.next().await {
    ///         println!("Received event: {:?}", event);
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn stream(&self) -> InboxStream<DaemonTransportSpec> {
        InboxStream::new(self.sender.subscribe())
    }

    /// Sends a ping request to the daemon to verify active communication.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] if the transport fails, times out, or if the
    /// daemon responds with anything other than a `Pong`.    
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
    ///
    /// This method sends an `Up` request to the daemon and returns a stream
    /// of events, allowing you to track the progress of each service starting
    /// (e.g., pulling images, compilation, or health checks).
    ///
    /// # Returns
    ///
    /// Returns an [`InboxStream`] which yields [`DaemonEvent`]s related to
    /// the startup process.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] if the daemon is unreachable or if the
    /// initial command execution fails.    #[instrument(skip(self), name = "up")]
    pub async fn up(&self) -> Result<InboxStream<DaemonTransportSpec>, ClientError> {
        self.execute_with_stream(DaemonRequest::Up).await
    }

    /// Stops all services managed by the daemon.
    ///
    /// This method sends a `Down` request to gracefully terminate all
    /// active services. Like [`Self::up`], it provides a stream to monitor
    /// the shutdown sequence.
    ///
    /// # Returns
    ///
    /// Returns an [`InboxStream`] which yields [`DaemonEvent`]s related to
    /// the termination process.    #[instrument(skip(self), name = "down")]
    pub async fn down(&self) -> Result<InboxStream<DaemonTransportSpec>, ClientError> {
        self.execute_with_stream(DaemonRequest::Down).await
    }

    /// Fetches the current status of all managed services.
    ///
    /// Unlike `up` or `down`, this is a request-response operation that
    /// returns the immediate state of the workspace without opening a long-running stream.
    ///
    /// # Returns
    ///
    /// Returns a vector of [`ServiceStatusResponse`] containing details for each service,
    /// such as PID, uptime, and health status.
    ///
    /// # Errors
    ///
    /// Returns a [`ProtocolError::UnexpectedResponse`] if the daemon
    /// provides a response other than status data.    #[instrument(skip(self), name = "status")]
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
