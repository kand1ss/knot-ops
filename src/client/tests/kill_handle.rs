use knot_client::handles::KillHandle;
use knot_client::policies::PolicyConfig;
use knot_sys::{Process, ProcessError};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use knot_sys::process::PlatformHandle;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);
const BINARY: &str = "client-fixture";

/// Resolve the process-fixture binary produced by Cargo.
///
/// `CARGO_BIN_EXE_client-process-fixture` is available for integration tests
/// when the binary is declared in Cargo.toml.
fn fixture_binary() -> PathBuf {
    PathBuf::from(
        std::env::var(format!("CARGO_BIN_EXE_{BINARY}"))
            .expect("CARGO_BIN_EXE_<name> is set by Cargo during integration tests"),
    )
}

async fn spawn_fixture_process() -> Process {
    let path = fixture_binary();
    let mut child = Command::new(&path)
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn fixture");

    let pid = child.id().expect("fixture exited immediately");

    let stdout = child.stdout.take().expect("stdout not piped");
    let mut reader = BufReader::new(stdout).lines();
    let line = reader
        .next_line()
        .await
        .expect("failed reading readiness signal")
        .expect("fixture exited before signaling readiness");
    assert_eq!(line, "ready", "unexpected fixture output: {line}");

    drop(child);

    Process::bind(pid, path.file_name().unwrap().to_string_lossy().to_string())
        .await
        .expect("failed to bind to fixture process")
}

async fn wait_until_process_exits(pid: u32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let exists = tokio::task::spawn_blocking(move || {
                use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

                let sys_pid = Pid::from(pid as usize);

                let mut system = System::new();

                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[sys_pid]),
                    false,
                    ProcessRefreshKind::nothing(),
                );

                system.process(sys_pid).is_some()
            })
            .await
            .expect("process inspection task panicked");

            if !exists {
                return;
            }

            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("process did not terminate within timeout");
}

/// Returns the process name reported by sysinfo.
///
/// This is used only to poll whether the process is still alive.
#[cfg(unix)]
fn current_process_name(pid: u32) -> String {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let sys_pid = Pid::from(pid as usize);

    let mut system = System::new();

    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sys_pid]),
        false,
        ProcessRefreshKind::nothing(),
    );

    system
        .process(sys_pid)
        .map(|process| process.name().to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(windows)]
fn process_exists() -> bool {
    // Process::spawn/kill is already exercised by the test. For Windows
    // we avoid depending on Unix-specific process inspection.
    true
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn create_handle(
    runtime_dir: &Path,
    process: Process,
    daemon_path: PathBuf,
) -> KillHandle<PlatformHandle> {
    KillHandle {
        runtime_dir: runtime_dir.to_path_buf(),
        process,
        daemon_path,
        policy: Arc::new(PolicyConfig::default()),
    }
}

#[tokio::test]
async fn kill_terminates_real_process() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let process = spawn_fixture_process().await;
    let pid = process.pid();

    #[cfg(windows)]
    {
        assert!(
            crate::process_exists(),
            "fixture process must be running before kill"
        );
    }

    #[cfg(unix)]
    {
        assert!(
            process_exists(pid),
            "fixture process must be running before kill"
        );
    }

    let handle = create_handle(temp_dir.path(), process, daemon_path);

    let stale_handle = handle
        .kill()
        .await
        .expect("kill should terminate the real process");

    assert_eq!(stale_handle.runtime_dir, temp_dir.path());

    wait_until_process_exits(pid).await;
}

#[tokio::test]
async fn kill_returns_stale_handle_with_original_runtime_dir() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let process = spawn_fixture_process().await;
    let pid = process.pid();

    let handle = create_handle(temp_dir.path(), process, daemon_path.clone());
    let stale_handle = handle.kill().await.expect("kill should succeed");

    wait_until_process_exits(pid).await;

    assert_eq!(stale_handle.runtime_dir, temp_dir.path());
    assert_eq!(stale_handle.daemon_path, daemon_path);
}

#[tokio::test]
async fn kill_preserves_policy_in_returned_stale_handle() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let process = spawn_fixture_process().await;
    let pid = process.pid();

    let policy = Arc::new(PolicyConfig::default());

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process,
        daemon_path,
        policy: Arc::clone(&policy),
    };

    let stale_handle = handle.kill().await.expect("kill should succeed");

    wait_until_process_exits(pid).await;

    assert!(
        Arc::ptr_eq(&stale_handle.policy, &policy),
        "kill must preserve the policy Arc"
    );
}

#[tokio::test]
async fn kill_is_idempotent_when_process_already_exited() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");
    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let process = spawn_fixture_process().await;
    let pid = process.pid();
    process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("pre-kill for setup should succeed");
    wait_until_process_exits(pid).await;

    let handle = create_handle(temp_dir.path(), process, daemon_path);
    let result = handle.kill().await;

    assert!(
        result.is_ok(),
        "KillHandle::kill() on an already-exited process must succeed idempotently, \
     not error — got: {result:?}"
    );
}

#[tokio::test]
async fn bind_returns_mismatch_when_process_name_differs() {
    let process = spawn_fixture_process().await;
    let pid = process.pid();

    let result = Process::bind(pid, "definitely-not-sleep".to_string()).await;

    match result {
        Err(ProcessError::Mismatch {
            expected,
            actual: _actual,
        }) => {
            assert_eq!(expected, "definitely-not-sleep");
            #[cfg(unix)]
            assert_eq!(_actual, BINARY);
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }

    process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("cleanup kill should succeed");
    wait_until_process_exits(pid).await;
}

#[tokio::test]
async fn bind_succeeds_when_name_matches_case_insensitively() {
    let process = spawn_fixture_process().await;
    let pid = process.pid();

    #[cfg(unix)]
    let bound = Process::bind(pid, BINARY.to_uppercase().to_string()).await;
    #[cfg(windows)]
    let bound = Process::bind(pid, BINARY.to_uppercase().to_string()).await;

    assert!(bound.is_ok(), "case-insensitive match must succeed");

    process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("cleanup kill should succeed");
    wait_until_process_exits(pid).await;
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_times_out_when_process_ignores_sigterm() {
    let process = spawn_fixture_process().await;

    let timeout = Duration::from_secs(1);
    let start = std::time::Instant::now();

    let result = process
        .terminate(timeout)
        .await
        .unwrap_or_else(|error| panic!("terminate() must not return an error: {error:?}"));

    let elapsed = start.elapsed();

    assert!(
        !result,
        "terminate() must return false when SIGTERM is ignored"
    );

    let slack = Duration::from_millis(50);
    assert!(
        elapsed + slack >= timeout,
        "terminate() returned too early: elapsed={elapsed:?}, timeout={timeout:?}"
    );
    assert!(
        elapsed < timeout + Duration::from_secs(2),
        "terminate() took suspiciously long: elapsed={elapsed:?}"
    );

    process
        .kill(timeout)
        .await
        .expect("kill should succeed as cleanup");
    wait_until_process_exits(process.pid()).await;
}

#[cfg(unix)]
#[tokio::test]
async fn kill_terminates_process_even_if_it_traps_sigterm() {
    let process = Process::spawn_with_args(
        Path::new("/bin/sh"),
        &["-c", "trap 'echo caught' TERM; sleep 5"],
    )
    .await
    .expect("failed to spawn trapping fixture");
    let pid = process.pid();

    process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("kill should succeed");

    wait_until_process_exits(pid).await;
}

#[tokio::test]
async fn spawn_returns_error_for_nonexistent_binary() {
    let result = Process::spawn(Path::new("/definitely/does/not/exist/binary")).await;
    assert!(result.is_err(), "spawning nonexistent binary must fail");
}

#[cfg(unix)]
#[tokio::test]
async fn kill_handle_escalates_to_real_sigkill_for_trapping_process() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");
    let timeout = Duration::from_millis(200);

    let process = spawn_fixture_process().await;
    let pid = process.pid();

    let mut policy = PolicyConfig::default();
    policy.kill.graceful_timeout = timeout;

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process,
        daemon_path: PathBuf::from("/bin/sh"),
        policy: Arc::new(policy),
    };

    let stale_handle = handle
        .kill()
        .await
        .expect("kill must escalate past a trapped SIGTERM and succeed via SIGKILL");

    wait_until_process_exits(pid).await;
    assert_eq!(stale_handle.runtime_dir, temp_dir.path());
}

#[cfg(unix)]
#[tokio::test]
async fn kill_returns_io_error_for_unkillable_pid() {
    let _temp_dir = TempDir::new().expect("failed to create temporary directory");
    let process = Process::bind(1, current_process_name(1)).await.unwrap();
    let result = process.kill(DEFAULT_TIMEOUT).await;
    assert!(
        result.is_err(),
        "binding to pid 1 should fail (wrong name or permission) — \
         documents that this path is NOT exercised end-to-end for PermissionDenied; \
         see kill.rs mock tests for the Io(PermissionDenied) propagation path instead"
    );
}

#[tokio::test]
async fn kill_handle_zero_timeout_sends_sigkill_immediately() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let process = spawn_fixture_process().await;
    let pid = process.pid();

    let mut policy = PolicyConfig::default();
    policy.kill.graceful_timeout = Duration::ZERO;

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process,
        #[cfg(unix)]
        daemon_path: PathBuf::from("/bin/sh"),
        #[cfg(windows)]
        daemon_path: PathBuf::from(r"C:\Windows\System32\ping.exe"),
        policy: Arc::new(policy),
    };

    let start = std::time::Instant::now();
    handle
        .kill()
        .await
        .expect("kill must succeed via immediate SIGKILL");
    let elapsed = start.elapsed();

    wait_until_process_exits(pid).await;
    assert!(
        elapsed < Duration::from_millis(100),
        "zero-timeout path must not incur any graceful-phase wait; elapsed={:?}",
        elapsed
    );
}
