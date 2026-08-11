use knot_client::ClientBuilder;
use knot_client::states::ConnectState;
use knot_core::consts::KNOT_DAEMON_LOCK_FILE;

#[cfg(unix)]
use knot_client::process::{Process, ProcessControl};
#[cfg(unix)]
use knot_core::consts::KNOT_SOCKET_FILE;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use tokio::time::{Duration, sleep};

/// Returns a system executable that can be kept alive long enough for the test.
///
/// The important property is that the process name reported by `sysinfo` is
/// deterministic and can therefore be passed to `ClientBuilder::with_expected_daemon_name`.
fn fixture_process() -> (PathBuf, Vec<String>, &'static str) {
    #[cfg(unix)]
    {
        // `sleep 60` remains alive long enough for the test and is available
        // on standard Unix environments used by CI.
        (PathBuf::from("/bin/sleep"), vec!["60".to_string()], "sleep")
    }

    #[cfg(windows)]
    {
        // PowerShell is available on supported Windows runners.
        //
        // `-NoProfile -Command Start-Sleep -Seconds 60` keeps the process
        // alive without creating any temporary executable/script file.
        (
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 60".to_string(),
            ],
            "powershell.exe",
        )
    }
}

/// Waits until the given PID no longer exists.
async fn wait_until_process_exits(pid: u32) {
    for _ in 0..250 {
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
        // `kill -0` checks whether the process exists without sending
        // a termination signal.
        match Command::new("kill").args(["-0", &pid.to_string()]).status() {
            Ok(status) => status.success(),
            Err(_) => false,
        }
    }

    #[cfg(windows)]
    {
        // `tasklist` is available on Windows CI runners.
        //
        // We deliberately check the PID rather than the process name because
        // the latter is irrelevant to this helper.
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

    let (daemon_path, args, expected_name) = fixture_process();

    let process =
        Process::spawn_with_args(&daemon_path, &args).expect("failed to spawn fixture process");

    let pid = process.pid();

    println!("fixture pid: {pid}");
    println!("fixture path: {}", daemon_path.display());

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    // Make the client believe that the daemon is running.
    tokio::fs::write(&lock_path, pid.to_string())
        .await
        .expect("failed to create daemon lock file");

    // The socket exists but is invalid. ConnectedHandle::new() must fail,
    // causing ClientBuilder to inspect the process table.
    tokio::fs::write(&socket_path, b"not a unix socket")
        .await
        .expect("failed to create fake socket");

    let result = ClientBuilder::new()
        .with_daemon_path(&daemon_path)
        .with_expected_daemon_name(expected_name)
        .with_runtime_dir(&runtime_dir)
        .connect()
        .await
        .expect("connect should resolve daemon state");

    let handle = match result {
        ConnectState::Hung(handle) => handle,

        other => {
            // Prevent the system process from leaking if the state machine
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
        "socket file must be removed after stale cleanup"
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
    tokio::fs::write(&lock_path, "4294967295")
        .await
        .expect("failed to create stale lock file");

    #[cfg(unix)]
    {
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
        tokio::fs::write(&socket_path, b"not a unix socket")
            .await
            .expect("failed to create fake socket");
    }

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
    {
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
        assert!(!socket_path.exists(), "stale socket must be removed");
    }
}

#[tokio::test]
async fn connect_returns_stale_when_lock_file_is_corrupted() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().join("runtime");

    tokio::fs::create_dir_all(&runtime_dir)
        .await
        .expect("failed to create runtime directory");

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);
    tokio::fs::write(&lock_path, "not-a-pid")
        .await
        .expect("failed to create corrupted lock file");

    #[cfg(unix)]
    {
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
        tokio::fs::write(&socket_path, b"not a unix socket")
            .await
            .expect("failed to create fake socket");
    }

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
    {
        let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
        assert!(!socket_path.exists());
    }
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
