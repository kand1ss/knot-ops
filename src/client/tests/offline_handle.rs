use knot_client::errors::{ClientError, DaemonLifecycleError};
use knot_client::handles::OfflineHandle;
use knot_client::policies::PolicyConfig;
use knot_core::consts::KNOT_SOCKET_FILE;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn create_dummy_binary(dir: &TempDir, name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let bin_path = dir.path().join(name);

        fs::write(
            &bin_path,
            r#"#!/bin/sh
while true; do
    sleep 60
done
"#,
        )
        .expect("failed to create dummy daemon");

        let mut permissions = fs::metadata(&bin_path)
            .expect("failed to stat dummy daemon")
            .permissions();

        permissions.set_mode(0o755);

        fs::set_permissions(&bin_path, permissions)
            .expect("failed to make dummy daemon executable");

        bin_path
    }

    #[cfg(windows)]
    {
        let bin_path = dir.path().join(format!("{name}.bat"));

        fs::write(
            &bin_path,
            r#"@echo off
:loop
ping -n 60 127.0.0.1 > nul
goto loop
"#,
        )
        .expect("failed to create dummy daemon");

        bin_path
    }
}

fn create_missing_binary_path(dir: &TempDir) -> PathBuf {
    #[cfg(unix)]
    {
        dir.path().join("non_existent_binary")
    }

    #[cfg(windows)]
    {
        dir.path().join("non_existent_binary.exe")
    }
}

fn create_handle(runtime_dir: &TempDir, daemon_path: PathBuf) -> OfflineHandle {
    OfflineHandle {
        runtime_dir: runtime_dir.path().to_path_buf(),
        daemon_path,
        policy: Arc::new(PolicyConfig::default()),
    }
}

#[cfg(unix)]
async fn spawn_mock_socket_server(socket_path: PathBuf) {
    use tokio::net::UnixListener;

    let _listener = UnixListener::bind(&socket_path).expect("failed to bind mock Unix socket");

    // Keep the socket alive long enough for launch() to observe it.
    tokio::time::sleep(Duration::from_secs(10)).await;
}

#[cfg(windows)]
async fn spawn_mock_socket_server(socket_path: PathBuf) {
    use tokio::net::TcpListener;

    let _listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock TCP listener");

    // The Windows implementation uses the socket path as the readiness
    // marker, so create the corresponding marker file.
    fs::write(&socket_path, "mock_socket").expect("failed to create mock socket marker");

    tokio::time::sleep(Duration::from_secs(10)).await;
}

#[tokio::test]
async fn test_launch_timeout_when_socket_never_appears() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");
    let daemon_path = create_dummy_binary(&temp_dir, "fake_daemon");

    let handle = create_handle(&temp_dir, daemon_path);

    let result = handle.launch().await;

    match result {
        Err(ClientError::Daemon(DaemonLifecycleError::LaunchFailed { message, .. })) => {
            assert!(
                message.contains("socket never appeared"),
                "unexpected error message: {message}"
            );
        }

        result => {
            panic!("expected LaunchFailed, got: {result:?}");
        }
    }
}

#[tokio::test]
async fn test_launch_fails_if_binary_does_not_exist() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");
    let daemon_path = create_missing_binary_path(&temp_dir);

    let handle = create_handle(&temp_dir, daemon_path);

    let result = handle.launch().await;

    assert!(
        matches!(
            result,
            Err(ClientError::Daemon(
                DaemonLifecycleError::LaunchFailed { .. }
            ))
        ),
        "expected LaunchFailed for missing binary, got: {result:?}"
    );
}

#[tokio::test]
async fn test_launch_waits_for_socket_to_appear() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    let socket_path = temp_dir.path().join(KNOT_SOCKET_FILE);
    let daemon_path = create_dummy_binary(&temp_dir, "fake_daemon");

    let socket_path_clone = socket_path.clone();

    tokio::spawn(async move {
        // Delay the endpoint enough to guarantee that launch() has to
        // perform at least several readiness checks.
        tokio::time::sleep(Duration::from_millis(500)).await;

        spawn_mock_socket_server(socket_path_clone).await;
    });

    let handle = create_handle(&temp_dir, daemon_path);

    let started_at = tokio::time::Instant::now();

    let result = handle.launch().await;

    let elapsed = started_at.elapsed();

    assert!(
        result.is_ok(),
        "expected launch to succeed, got: {result:?}"
    );

    assert!(
        elapsed >= Duration::from_millis(400),
        "launch returned before the socket was expected to appear: {elapsed:?}"
    );
}

#[tokio::test]
async fn test_launch_fails_when_daemon_exits_before_socket_appears() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    #[cfg(unix)]
    let daemon_path = {
        let path = temp_dir.path().join("exiting_daemon");

        fs::write(
            &path,
            r#"#!/bin/sh
exit 1
"#,
        )
        .expect("failed to create exiting daemon");

        let mut permissions = fs::metadata(&path)
            .expect("failed to stat exiting daemon")
            .permissions();

        permissions.set_mode(0o755);

        fs::set_permissions(&path, permissions).expect("failed to make exiting daemon executable");

        path
    };

    #[cfg(windows)]
    let daemon_path = {
        let path = temp_dir.path().join("exiting_daemon.bat");

        fs::write(
            &path,
            r#"@echo off
exit /b 1
"#,
        )
        .expect("failed to create exiting daemon");

        path
    };

    let handle = create_handle(&temp_dir, daemon_path);

    let result = handle.launch().await;

    assert!(
        result.is_err(),
        "launch must fail when daemon exits before socket appears"
    );
}

#[tokio::test]
async fn test_launch_does_not_treat_regular_file_as_socket() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    let socket_path = temp_dir.path().join(KNOT_SOCKET_FILE);

    fs::write(&socket_path, "this is not a socket").expect("failed to create fake socket file");

    let daemon_path = create_dummy_binary(&temp_dir, "fake_daemon");

    let handle = create_handle(&temp_dir, daemon_path);

    let result = handle.launch().await;

    assert!(
        result.is_err(),
        "launch must not accept a regular file as a valid socket"
    );
}

#[tokio::test]
async fn test_launch_succeeds_when_socket_appears_after_polling() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    let socket_path = temp_dir.path().join(KNOT_SOCKET_FILE);
    let daemon_path = create_dummy_binary(&temp_dir, "fake_daemon");

    let socket_path_clone = socket_path.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;

        spawn_mock_socket_server(socket_path_clone).await;
    });

    let handle = create_handle(&temp_dir, daemon_path);

    let result = handle.launch().await;

    assert!(
        result.is_ok(),
        "expected launch to succeed after socket appears, got: {result:?}"
    );

    assert!(
        socket_path.exists(),
        "socket should exist after successful launch"
    );
}
