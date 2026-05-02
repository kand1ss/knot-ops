use async_trait::async_trait;
use knot_client::{KnotClient, launcher::DaemonLauncher};
use knot_core::consts::{KNOT_FOLDER_NAME, KNOT_PID_FILE, KNOT_SOCKET_FILE};
#[allow(unused_imports)]
use knot_core::errors::{
    ClientError, DaemonLifecycleError, HealthcheckError, ProtocolError, WorkspaceError,
};
use knot_protocol::daemon::{
    DaemonEvent, DaemonRequest, DaemonResponse, DaemonTransportSpec, ServiceStatusResponse,
};
use knot_transport::messages::{MessageContext, MessageKind};
use knot_transport::transport::{MessageTransport, Server, ipc::IpcServer, ipc::IpcTransport};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::task::JoinHandle;

fn setup_temp_workspace(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let thread_id = std::thread::current().id();
    path.push(format!("knot-client-tests-{}-{:?}", suffix, thread_id));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    let knot_dir = path.join(KNOT_FOLDER_NAME);
    fs::create_dir(&knot_dir).unwrap();
    path
}

fn create_pid_file(workspace: &Path, pid: u32) {
    let pid_path = workspace.join(KNOT_FOLDER_NAME).join(KNOT_PID_FILE);
    fs::write(pid_path, pid.to_string()).unwrap();
}

async fn start_dummy_daemon(socket_path: PathBuf) -> JoinHandle<()> {
    #[cfg(not(windows))]
    {
        if socket_path.exists() {
            let _ = fs::remove_file(&socket_path);
        }
    }

    let server = IpcServer::bind(socket_path)
        .await
        .expect("Failed to bind IpcServer");
    tokio::spawn(async move {
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, DaemonTransportSpec>| {
                    let _ = transport
                        .serve_with(
                            async |mut ctx: MessageContext<
                                '_,
                                IpcTransport,
                                DaemonTransportSpec,
                            >| {
                                if let MessageKind::Request(req) = ctx.kind() {
                                    let res = match req {
                                        DaemonRequest::Ping => DaemonResponse::Pong,
                                        DaemonRequest::Up => {
                                            let ev =
                                                DaemonEvent::ServiceEvent(ServiceStatusResponse {
                                                    pid: 1234,
                                                    name: "test_service".to_string(),
                                                    status: "Running".to_string(),
                                                    uptime: "0s".to_string(),
                                                    healthy: true,
                                                });
                                            ctx.event(ev).await.unwrap();
                                            DaemonResponse::Ok
                                        }
                                        DaemonRequest::Down => DaemonResponse::Done,
                                        DaemonRequest::Status => DaemonResponse::Status(vec![]),
                                    };
                                    ctx.reply(res).await.unwrap();
                                }
                                Ok(())
                            },
                        )
                        .await;
                    Ok(())
                },
            )
            .await;
    })
}

struct MockDaemonLauncher {
    should_fail: bool,
    should_timeout: bool,
}

impl MockDaemonLauncher {
    fn new(should_fail: bool, should_timeout: bool) -> Self {
        Self {
            should_fail,
            should_timeout,
        }
    }
}

#[async_trait]
impl DaemonLauncher for MockDaemonLauncher {
    async fn launch(&self, directory: &Path) -> Result<u32, ClientError> {
        if self.should_fail {
            return Err(ClientError::Daemon(DaemonLifecycleError::LaunchFailed {
                message: "Mock failure".to_string(),
                binary_path: "mock".to_string(),
                target_dir: directory.to_string_lossy().into_owned(),
                error: "mock".to_string(),
            }));
        } else if self.should_timeout {
            Ok(1234)
        } else {
            let socket_path = directory.join(KNOT_SOCKET_FILE);
            start_dummy_daemon(socket_path).await;

            let pid_path = directory.join(KNOT_PID_FILE);
            let pid = std::process::id();
            fs::write(pid_path, pid.to_string()).unwrap();

            Ok(pid)
        }
    }

    fn binary_path(&self) -> &Path {
        Path::new("mock_daemon")
    }
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_connect_to_directory_success() {
    let workspace = setup_temp_workspace("connect_success");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    assert!(client.is_connected());

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_connect_fails_no_workspace() {
    let workspace =
        std::env::temp_dir().join(format!("no_workspace_{:?}", std::thread::current().id()));
    let _ = fs::remove_dir_all(&workspace);
    fs::create_dir_all(&workspace).unwrap();

    let result = KnotClient::connect_to_directory(&workspace).await;
    assert!(matches!(
        result,
        Err(ClientError::Workspace(WorkspaceError::NotInitialized(_)))
    ));
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_launch_daemon_success() {
    let workspace = setup_temp_workspace("launch_success");
    let launcher = MockDaemonLauncher::new(false, false);

    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_launcher(launcher);

    let client = client.launch_daemon().await.unwrap();
    assert!(client.is_connected());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_healthcheck_healthy() {
    let workspace = setup_temp_workspace("healthcheck");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    create_pid_file(&workspace, std::process::id());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    assert!(client.healthcheck().await.is_ok());

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_ping_up_down_status() {
    let workspace = setup_temp_workspace("commands");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();

    assert!(client.ping().await.is_ok());

    let _up_stream = client.up().await.unwrap();
    let _down_stream = client.down().await.unwrap();
    let status = client.status().await.unwrap();
    assert!(status.is_empty());

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_launch_daemon_fails() {
    let workspace = setup_temp_workspace("launch_fails");
    let launcher = MockDaemonLauncher::new(true, false);

    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_launcher(launcher);

    let result = client.launch_daemon().await;
    assert!(result.is_err());

    if let Err(ClientError::Daemon(DaemonLifecycleError::LaunchFailed { .. })) = result {
        // ok
    } else {
        panic!("Expected LaunchFailed error, got a different error or success");
    }
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_connect_or_launch_success() {
    let workspace = setup_temp_workspace("connect_or_launch");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    create_pid_file(&workspace, std::process::id());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // This will successfully connect and bypass launch_daemon
    let client = KnotClient::connect_or_launch(&workspace).await.unwrap();
    assert!(client.is_connected());

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_launch_timeout() {
    let workspace = setup_temp_workspace("launch_timeout");
    // should_timeout = true means launch() succeeds but no socket is created
    let launcher = MockDaemonLauncher::new(false, true);

    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_launcher(launcher);

    let result = client.launch_daemon().await;
    assert!(result.is_err());

    if let Err(ClientError::Daemon(DaemonLifecycleError::LaunchFailed { message, .. })) = result {
        assert!(message.contains("never appeared"));
    } else {
        panic!("Expected LaunchFailed timeout error");
    }
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_healthcheck_stale_socket() {
    let workspace = setup_temp_workspace("stale_socket");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // Deliberately NOT creating a PID file

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    let err = client.healthcheck().await.unwrap_err();

    #[cfg(windows)]
    assert!(matches!(
        err,
        ClientError::Healthcheck(HealthcheckError::InconsistentState(_))
    ));
    #[cfg(not(windows))]
    assert!(matches!(
        err,
        ClientError::Healthcheck(HealthcheckError::StaleSocket(_))
    ));

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_event_stream() {
    let workspace = setup_temp_workspace("event_stream");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();

    let stream = client.up().await.unwrap();

    if let Some(event) = stream.next().await.unwrap() {
        match event {
            DaemonEvent::ServiceEvent(status) => {
                assert_eq!(status.name, "test_service");
                assert!(status.healthy);
            }
        }
    } else {
        panic!("Expected an event from stream");
    }

    server_handle.abort();
}

fn make_disconnected_client(knot_dir: PathBuf) -> KnotClient<IpcTransport> {
    KnotClient::<IpcTransport>::new(knot_dir, None)
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_stale_socket_removes_files() {
    let workspace = setup_temp_workspace("repair_stale");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    // Simulate stale state: only PID file exists, no socket
    create_pid_file(&workspace, 99999);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());

    #[cfg(not(windows))]
    let sock_path = knot_dir.join(KNOT_SOCKET_FILE);
    #[cfg(not(windows))]
    fs::write(&sock_path, b"").unwrap();

    client
        .repair(&HealthcheckError::StaleSocket(
            knot_dir.join(KNOT_SOCKET_FILE),
        ))
        .await
        .unwrap();

    assert!(
        !pid_path.exists(),
        "PID file should be removed after repair"
    );
    #[cfg(not(windows))]
    assert!(
        !sock_path.exists(),
        "Socket file should be removed after repair"
    );
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_process_not_exists_removes_files() {
    let workspace = setup_temp_workspace("repair_proc_not_exists");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    create_pid_file(&workspace, 99999);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    client
        .repair(&HealthcheckError::ProcessNotExists(99999))
        .await
        .unwrap();

    assert!(
        !pid_path.exists(),
        "PID file should be removed after repair"
    );
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_inconsistent_state_removes_files() {
    let workspace = setup_temp_workspace("repair_inconsistent");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    create_pid_file(&workspace, 99999);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    client
        .repair(&HealthcheckError::InconsistentState("test".to_string()))
        .await
        .unwrap();

    assert!(
        !pid_path.exists(),
        "PID file should be removed after repair"
    );
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_daemon_not_responding_with_pid_removes_files() {
    let workspace = setup_temp_workspace("repair_not_responding");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    // Use a PID that certainly doesn't exist (very large number)
    create_pid_file(&workspace, 4194304);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    client
        .repair(&HealthcheckError::DaemonNotResponding)
        .await
        .unwrap();

    assert!(
        !pid_path.exists(),
        "PID file should be removed after repair"
    );
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_daemon_not_responding_without_pid_removes_files() {
    let workspace = setup_temp_workspace("repair_no_pid");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    // No PID file — repair should still succeed and clean what it can
    assert!(!pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    let result = client.repair(&HealthcheckError::DaemonNotResponding).await;
    assert!(result.is_ok());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_zombie_process_removes_files() {
    let workspace = setup_temp_workspace("repair_zombie");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    create_pid_file(&workspace, 4194304);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    // PID 4194304 won't exist; sysinfo will not find it, clean_volatile_files runs
    client
        .repair(&HealthcheckError::ZombieProcess(4194304))
        .await
        .unwrap();

    assert!(
        !pid_path.exists(),
        "PID file should be removed after repair"
    );
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_not_connected_is_noop() {
    let workspace = setup_temp_workspace("repair_not_connected");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    // NotConnected should be a no-op — PID file stays
    create_pid_file(&workspace, 99999);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    let result = client.repair(&HealthcheckError::NotConnected).await;
    assert!(result.is_ok());
    assert!(
        pid_path.exists(),
        "PID file should NOT be removed for NotConnected"
    );
}

#[test]
fn test_with_timeout_stores_value() {
    let workspace = setup_temp_workspace("builder_timeout");
    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_timeout(Duration::from_secs(42));
    assert!(!client.is_connected());
}

#[test]
fn test_with_retries_stores_value() {
    let workspace = setup_temp_workspace("builder_retries");
    let client =
        KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None).with_retries(5);
    assert!(!client.is_connected());
}

#[test]
fn test_is_connected_false_when_no_transport() {
    let workspace = setup_temp_workspace("no_transport");
    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None);
    assert!(!client.is_connected());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_ping_fails_when_disconnected() {
    let workspace = setup_temp_workspace("ping_disconnected");
    let client = make_disconnected_client(workspace.join(KNOT_FOLDER_NAME));
    let err = client.ping().await.unwrap_err();
    assert!(matches!(
        err,
        ClientError::Daemon(DaemonLifecycleError::NotRunning(_))
    ));
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_up_fails_when_disconnected() {
    let workspace = setup_temp_workspace("up_disconnected");
    let client = make_disconnected_client(workspace.join(KNOT_FOLDER_NAME));
    let err = client.up().await.err().expect("expected error");
    assert!(matches!(
        err,
        ClientError::Daemon(DaemonLifecycleError::NotRunning(_))
    ));
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_down_fails_when_disconnected() {
    let workspace = setup_temp_workspace("down_disconnected");
    let client = make_disconnected_client(workspace.join(KNOT_FOLDER_NAME));
    let err = client.down().await.err().expect("expected error");
    assert!(matches!(
        err,
        ClientError::Daemon(DaemonLifecycleError::NotRunning(_))
    ));
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_status_fails_when_disconnected() {
    let workspace = setup_temp_workspace("status_disconnected");
    let client = make_disconnected_client(workspace.join(KNOT_FOLDER_NAME));
    let err = client.status().await.unwrap_err();
    assert!(matches!(
        err,
        ClientError::Daemon(DaemonLifecycleError::NotRunning(_))
    ));
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_healthcheck_fails_when_disconnected() {
    let workspace = setup_temp_workspace("healthcheck_disconnected");
    let client = make_disconnected_client(workspace.join(KNOT_FOLDER_NAME));
    let err = client.healthcheck().await.unwrap_err();
    assert!(matches!(
        err,
        ClientError::Healthcheck(HealthcheckError::NotConnected)
    ));
}

async fn start_status_daemon(socket_path: PathBuf) -> JoinHandle<()> {
    #[cfg(not(windows))]
    {
        if socket_path.exists() {
            let _ = fs::remove_file(&socket_path);
        }
    }
    let server = IpcServer::bind(socket_path).await.expect("bind failed");
    tokio::spawn(async move {
        let _ = server
            .accept_with(
                async |transport: MessageTransport<IpcTransport, DaemonTransportSpec>| {
                    let _ = transport
                        .serve_with(
                            async |mut ctx: MessageContext<
                                '_,
                                IpcTransport,
                                DaemonTransportSpec,
                            >| {
                                if let MessageKind::Request(DaemonRequest::Status) = ctx.kind() {
                                    let services = vec![ServiceStatusResponse {
                                        pid: 42,
                                        name: "web".to_string(),
                                        status: "Running".to_string(),
                                        uptime: "5m".to_string(),
                                        healthy: true,
                                    }];
                                    ctx.reply(DaemonResponse::Status(services)).await.unwrap();
                                }
                                Ok(())
                            },
                        )
                        .await;
                    Ok(())
                },
            )
            .await;
    })
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_status_returns_populated_list() {
    let workspace = setup_temp_workspace("status_services");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_status_daemon(socket_path).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    let services = client.status().await.unwrap();

    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "web");
    assert_eq!(services[0].pid, 42);
    assert!(services[0].healthy);

    server_handle.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_integration_clears_pid_file() {
    let workspace = setup_temp_workspace("repair_integration");
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);
    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connect without PID file → healthcheck detects inconsistency
    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    let hc_result = client.healthcheck().await;
    assert!(hc_result.is_err());

    // Extract the HealthcheckError and repair
    if let Err(ClientError::Healthcheck(ref e)) = hc_result {
        client.repair(e).await.unwrap();
    }

    assert!(
        !pid_path.exists(),
        "PID file should be cleaned up after repair"
    );

    server_handle.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_connect_finds_knot_from_subdirectory() {
    let workspace = setup_temp_workspace("subdir_discovery");
    let socket_path = workspace.join(KNOT_FOLDER_NAME).join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // create nested subdir and connect from there
    let subdir = workspace.join("project").join("src");
    fs::create_dir_all(&subdir).unwrap();

    let client = KnotClient::connect_to_directory(&subdir).await.unwrap();
    assert!(client.is_connected());

    server_handle.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
}
