use knot_client::ClientBuilder;
use knot_client::process::{Process, ProcessControl};
use knot_client::states::ConnectState;
use knot_core::consts::{KNOT_DAEMON_LOCK_FILE, KNOT_SOCKET_FILE};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use tokio::time::{Duration, sleep};

/// Creates a small executable fixture that stays alive until killed.
///
/// The executable name is intentionally configurable because `ClientBuilder`
/// validates the process name before creating a `KillHandle`.
fn create_dummy_binary(temp_dir: &TempDir, name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_dir.path().join(name);

        std::fs::write(
            &path,
            br#"#!/bin/sh
trap 'exit 0' TERM INT
while true; do
    sleep 1
done
"#,
        )
        .expect("failed to write fixture executable");

        let mut permissions = std::fs::metadata(&path)
            .expect("failed to stat fixture executable")
            .permissions();

        permissions.set_mode(0o755);

        std::fs::set_permissions(&path, permissions).expect("failed to make fixture executable");

        path
    }

    #[cfg(windows)]
    {
        let path = temp_dir.path().join(format!("{name}.cmd"));

        std::fs::write(
            &path,
            "@echo off\r\n\
             :loop\r\n\
             timeout /t 1 /nobreak >nul\r\n\
             goto loop\r\n",
        )
        .expect("failed to write fixture executable");

        path
    }
}

/// Waits until the given PID no longer exists.
async fn wait_until_process_exits(pid: u32) {
    for _ in 0..100 {
        if !process_exists(pid) {
            return;
        }

        sleep(Duration::from_millis(20)).await;
    }

    panic!("process {pid} did not exit within timeout");
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // `kill -0` checks process existence without sending a signal.
        match Command::new("kill").args(["-0", &pid.to_string()]).status() {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    #[cfg(windows)]
    {
        match Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn connect_returns_hung_for_running_process_with_invalid_ipc() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let daemon_path = create_dummy_binary(&temp_dir, "knot_fixture");

    let process = Process::spawn(&daemon_path).expect("failed to spawn fixture process");

    let pid = process.pid();
    println!("fixture pid: {pid}");
    println!("fixture path: {}", daemon_path.display());

    let pid = process.pid();

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    // The lock file makes the client believe that a daemon is running.
    tokio::fs::write(&lock_path, pid.to_string())
        .await
        .expect("failed to create daemon lock file");

    // The socket exists but is not a real Unix socket.
    // ConnectedHandle::new() must therefore fail, and connect()
    // must continue with process-state inspection.
    tokio::fs::write(&socket_path, b"not a unix socket")
        .await
        .expect("failed to create fake socket");

    let result = ClientBuilder::new()
        .with_daemon_path(&daemon_path)
        .with_expected_daemon_name("knot_fixture")
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve daemon state");

    let handle = match result {
        ConnectState::Hung(handle) => handle,

        other => {
            // Make sure the fixture cannot leak if the state machine
            // unexpectedly returns another state.
            let _ = process.kill();

            panic!("expected ConnectState::Hung, got {other:?}");
        }
    };

    assert_eq!(handle.runtime_dir, runtime_dir);

    // KillHandle must terminate the real process and transition
    // the state machine to StaleHandle.
    let stale_handle = handle
        .kill()
        .expect("kill should terminate the real process");

    wait_until_process_exits(pid).await;

    assert_eq!(stale_handle.runtime_dir, runtime_dir);

    // StaleHandle::clean() must remove the orphaned artifacts and
    // transition to OfflineHandle.
    let offline_handle = stale_handle
        .clean()
        .await
        .expect("stale cleanup should succeed");

    assert_eq!(offline_handle.runtime_dir, runtime_dir);

    assert!(
        !lock_path.exists(),
        "lock file must be removed after stale cleanup"
    );

    assert!(
        !socket_path.exists(),
        "The socket file needs to be deleted following stale cleanup."
    );
}

#[tokio::test]
async fn connect_returns_stale_when_lock_file_contains_dead_pid() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    tokio::fs::write(&lock_path, "4294967295")
        .await
        .expect("failed to create stale lock file");

    #[cfg(unix)]
    tokio::fs::write(&socket_path, b"not a unix socket")
        .await
        .expect("failed to create fake socket");

    let result = ClientBuilder::new()
        .with_runtime_dir(&runtime_dir)
        .with_expected_daemon_name("knotd")
        .connect()
        .await
        .expect("connect should resolve stale state");

    let handle = match result {
        ConnectState::Stale(handle) => handle,

        other => {
            panic!("expected ConnectState::Stale, got {other:?}");
        }
    };

    assert_eq!(handle.runtime_dir, runtime_dir);

    let offline_handle = handle.clean().await.expect("stale cleanup should succeed");

    assert_eq!(offline_handle.runtime_dir, runtime_dir);

    assert!(!lock_path.exists(), "stale lock file must be removed");

    #[cfg(unix)]
    assert!(!socket_path.exists(), "stale socket must be removed");
}

#[tokio::test]
async fn connect_returns_stale_when_lock_file_is_corrupted() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    tokio::fs::write(&lock_path, "not-a-pid")
        .await
        .expect("failed to create corrupted lock file");

    #[cfg(unix)]
    tokio::fs::write(&socket_path, b"not a unix socket")
        .await
        .expect("failed to create fake socket");

    let result = ClientBuilder::new()
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve corrupted state");

    let handle = match result {
        ConnectState::Stale(handle) => handle,

        other => {
            panic!("expected ConnectState::Stale, got {other:?}");
        }
    };

    assert_eq!(handle.runtime_dir, runtime_dir);

    let offline_handle = handle.clean().await.expect("stale cleanup should succeed");

    assert_eq!(offline_handle.runtime_dir, runtime_dir);

    assert!(!lock_path.exists());

    #[cfg(unix)]
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn connect_returns_stale_when_only_lock_file_exists() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    tokio::fs::write(&lock_path, "12345")
        .await
        .expect("failed to create lock file");

    let result = ClientBuilder::new()
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve stale state");

    assert!(
        matches!(result, ConnectState::Stale(_)),
        "expected stale state when only lock file exists"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn connect_returns_stale_when_only_socket_exists() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    tokio::fs::write(&socket_path, b"orphaned socket")
        .await
        .expect("failed to create socket artifact");

    let result = ClientBuilder::new()
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve stale state");

    assert!(
        matches!(result, ConnectState::Stale(_)),
        "expected stale state when only socket exists"
    );
}

#[tokio::test]
async fn connect_returns_offline_when_no_daemon_artifacts_exist() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let result = ClientBuilder::new()
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve offline state");

    match result {
        ConnectState::Offline(handle) => {
            assert_eq!(handle.runtime_dir, runtime_dir);
        }

        other => {
            panic!("expected ConnectState::Offline, got {other:?}");
        }
    }
}
