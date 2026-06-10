use async_trait::async_trait;
use knot_client::{KnotClient, launcher::DaemonLauncher};
use knot_core::consts::{KNOT_FOLDER_NAME, KNOT_PID_FILE, KNOT_SOCKET_FILE};
#[allow(unused_imports)]
use knot_core::errors::{
    ClientError, DaemonLifecycleError, HealthcheckError, ProtocolError, WorkspaceError,
};
use knot_protocol::daemon::{
    DaemonEvent, DaemonRequest, DaemonResponse, DaemonTransportSpec, ServiceStatusResponse,
    TaskData, TaskStatus,
};
use knot_transport::messages::{Message, MessageContext, MessageKind};
use knot_transport::transport::RawTransport;
use knot_transport::transport::{MessageTransport, Server, ipc::IpcServer, ipc::IpcTransport};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::task::JoinHandle;

fn setup_temp_workspace() -> (tempfile::TempDir, PathBuf) {
    let temp_workspace = tempfile::Builder::new()
        .prefix("knot-test-")
        .tempdir()
        .expect("Failed to create temporary directory");

    let path = temp_workspace.path().to_path_buf();

    let knot_dir = path.join(KNOT_FOLDER_NAME);
    fs::create_dir(&knot_dir).expect("Failed to create .knot directory");

    (temp_workspace, path)
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

async fn setup_client_with_daemon(
    workspace: PathBuf,
) -> (Arc<KnotClient<IpcTransport>>, JoinHandle<()>) {
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);
    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    assert!(client.is_connected());
    (Arc::new(client), server_handle)
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_connect_to_directory_success() {
    let (_temp, workspace) = setup_temp_workspace();
    let socket_path = workspace.join(KNOT_FOLDER_NAME).join(KNOT_SOCKET_FILE);

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
    let (_dir, workspace) = setup_temp_workspace();
    let launcher = MockDaemonLauncher::new(false, false);

    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_launcher(launcher);

    let client = client.launch_daemon().await.unwrap();
    assert!(client.is_connected());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_healthcheck_healthy() {
    let (_dir, workspace) = setup_temp_workspace();
    create_pid_file(&workspace, std::process::id());
    let (client, handle) = setup_client_with_daemon(workspace).await;
    client.healthcheck().await.unwrap();
    handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_ping_up_down_status() {
    let (_dir, workspace) = setup_temp_workspace();
    let (client, handle) = setup_client_with_daemon(workspace).await;

    assert!(client.ping().await.is_ok());

    let _up_stream = client.up().await.unwrap();
    let _down_stream = client.down().await.unwrap();
    let status = client.status().await.unwrap();
    assert!(status.is_empty());

    handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_launch_daemon_fails() {
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    create_pid_file(&workspace, std::process::id());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = KnotClient::connect_or_launch(&workspace).await.unwrap();
    assert!(client.is_connected());

    server_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_launch_timeout() {
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

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
async fn test_inbox_stream() {
    let (_dir, workspace) = setup_temp_workspace();
    let (client, handle) = setup_client_with_daemon(workspace).await;

    let mut stream = client.up().await.unwrap();

    if let Ok(Some(DaemonEvent::ServiceEvent(status))) = stream.next().await {
        assert_eq!(status.name, "test_service");
        assert!(status.healthy);
    } else {
        panic!("Expected a ServiceEvent from stream, but got something else or None");
    };

    handle.abort();
}

fn make_disconnected_client(knot_dir: PathBuf) -> KnotClient<IpcTransport> {
    KnotClient::<IpcTransport>::new(knot_dir, None)
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_stale_socket_removes_files() {
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    assert!(!pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
    let result = client.repair(&HealthcheckError::DaemonNotResponding).await;
    assert!(result.is_ok());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_repair_zombie_process_removes_files() {
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    create_pid_file(&workspace, 4194304);
    assert!(pid_path.exists());

    let client = make_disconnected_client(knot_dir.clone());
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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

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
    let (_dir, workspace) = setup_temp_workspace();
    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None)
        .with_timeout(Duration::from_secs(42));
    assert!(!client.is_connected());
}

#[test]
fn test_with_retries_stores_value() {
    let (_dir, workspace) = setup_temp_workspace();
    let client =
        KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None).with_retries(5);
    assert!(!client.is_connected());
}

#[test]
fn test_is_connected_false_when_no_transport() {
    let (_dir, workspace) = setup_temp_workspace();
    let client = KnotClient::<IpcTransport>::new(workspace.join(KNOT_FOLDER_NAME), None);
    assert!(!client.is_connected());
}

#[tokio::test]
#[cfg_attr(windows, serial_test::serial)]
async fn test_ping_fails_when_disconnected() {
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
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
    let (_dir, workspace) = setup_temp_workspace();
    let knot_dir = workspace.join(KNOT_FOLDER_NAME);
    let pid_path = knot_dir.join(KNOT_PID_FILE);

    let socket_path = knot_dir.join(KNOT_SOCKET_FILE);
    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    let hc_result = client.healthcheck().await;
    assert!(hc_result.is_err());

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
    let (_dir, workspace) = setup_temp_workspace();
    let socket_path = workspace.join(KNOT_FOLDER_NAME).join(KNOT_SOCKET_FILE);

    let server_handle = start_dummy_daemon(socket_path).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let subdir = workspace.join("project").join("src");
    fs::create_dir_all(&subdir).unwrap();

    let client = KnotClient::connect_to_directory(&subdir).await.unwrap();
    assert!(client.is_connected());

    server_handle.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
}

use knot_transport::codec::{BinaryCodec, MessageCodec};
use knot_transport::test_utils::MockRaw;
use std::sync::Arc;
use tokio::task::JoinSet;

fn setup_test_client(
    path: PathBuf,
) -> (Arc<KnotClient<MockRaw>>, tokio::sync::mpsc::Sender<Vec<u8>>) {
    let (tx, rx) = tokio::sync::mpsc::channel(1024);
    let transport = MockRaw::new(rx, tx.clone());
    let client = KnotClient::new(path, Some(transport.to_messaged()));
    assert!(client.is_connected());
    (Arc::new(client), tx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(windows, serial_test::serial)]
async fn test_concurrent_stream_subscribers() {
    let (_dir, workspace) = setup_temp_workspace();
    let (client, mock_daemon_tx) = setup_test_client(workspace);
    let mut join_set = JoinSet::new();

    for i in 0..50 {
        let client_clone = Arc::clone(&client);
        join_set.spawn(async move {
            let mut stream = client_clone.stream();
            let mut received_count = 0;

            for _ in 0..10 {
                if let Ok(_event) = stream.next().await {
                    received_count += 1;
                }
            }
            assert_eq!(received_count, 10, "Task {} missed some events!", i);
        });
    }

    tokio::time::sleep(Duration::from_millis(10)).await;

    for i in 0..10 {
        let event: Message<DaemonRequest, DaemonResponse, DaemonEvent> = Message::event(
            0,
            DaemonEvent::TaskEvent(TaskData::new(format!("task_{}", i), TaskStatus::Running)),
        );
        mock_daemon_tx
            .send(BinaryCodec::encode(&event).unwrap())
            .await
            .unwrap();
    }

    while let Some(res) = join_set.join_next().await {
        res.expect("A subscriber task panicked");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(windows, serial_test::serial)]
async fn test_concurrent_requests_isolation() {
    let (_dir, workspace) = setup_temp_workspace();
    let (client, handle) = setup_client_with_daemon(workspace).await;
    let mut join_set = JoinSet::new();

    for _ in 0..100 {
        let client_clone = Arc::clone(&client);
        join_set.spawn(async move {
            let result = client_clone.ping().await;
            if let Err(e) = result {
                println!("{}", e);
                panic!("Request failed or received wrong response");
            };
        });
    }

    while let Some(res) = join_set.join_next().await {
        res.expect("A requester task panicked");
    }
    handle.abort();
}

async fn spawn_spam_server(socket_path: PathBuf, spam_count: u16) -> JoinSet<()> {
    #[cfg(not(windows))]
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    let mut set = JoinSet::new();

    let server = IpcServer::bind(socket_path)
        .await
        .expect("Failed to bind IpcServer");

    set.spawn(async move {
        let transport = server
            .accept()
            .await
            .unwrap()
            .to_messaged::<DaemonTransportSpec>();

        let transport = Arc::new(transport);
        let spam_transport = Arc::clone(&transport);

        tokio::spawn(async move {
            for i in 0..spam_count {
                let event = Message::event(
                    0,
                    DaemonEvent::TaskEvent(TaskData::new(
                        format!("task_{}", i),
                        TaskStatus::Running,
                    )),
                );
                spam_transport.send(event).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        while let Ok(ctx) = transport.recv().await {
            let (message, _) = ctx.into_parts();
            if let MessageKind::Request(req) = &message.kind {
                let res = match req {
                    DaemonRequest::Ping => DaemonResponse::Pong,
                    DaemonRequest::Status => DaemonResponse::Status(vec![]),
                    DaemonRequest::Down => DaemonResponse::Done,
                    DaemonRequest::Up => {
                        let ev = DaemonEvent::ServiceEvent(ServiceStatusResponse {
                            pid: 1234,
                            name: "test_service".to_string(),
                            status: "Running".to_string(),
                            uptime: "0s".to_string(),
                            healthy: true,
                        });
                        let ev_msg = Message::event(message.id(), ev);
                        transport.send(ev_msg).await.unwrap();
                        DaemonResponse::Ok
                    }
                };

                let reply_msg = Message::response(message.id(), res);
                transport.send(reply_msg).await.unwrap();
            }
        }
    });

    set
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg_attr(windows, serial_test::serial)]
async fn test_mixed_chaos_load() {
    let (_dir, workspace) = setup_temp_workspace();
    let socket_path = workspace.join(KNOT_FOLDER_NAME).join(KNOT_SOCKET_FILE);
    let mut handle = spawn_spam_server(socket_path, 200).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let client = KnotClient::connect_to_directory(&workspace).await.unwrap();
    let client = Arc::new(client);
    assert!(client.is_connected());
    let mut join_set = JoinSet::new();

    for _ in 0..10 {
        let client_clone = Arc::clone(&client);
        join_set.spawn(async move {
            for _ in 0..50 {
                let _ = client_clone.status().await;
            }
        });
    }

    for _ in 0..10 {
        let client_clone = Arc::clone(&client);
        join_set.spawn(async move {
            let mut stream = client_clone.stream();
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                while let Ok(Some(_event)) = stream.next().await {}
            })
            .await;
        });
    }

    let result = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(res) = join_set.join_next().await {
            res.expect("Task panicked during chaos test");
        }
    })
    .await;

    assert!(result.is_ok(), "Chaos test timed out (possible deadlock!)");
    handle.abort_all();
}
