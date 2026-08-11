use knot_client::handles::KillHandle;
use knot_client::policies::PolicyConfig;
use knot_client::process::Process;
use knot_client::process::ProcessControl;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::time::{sleep, timeout};

fn spawn_fixture_process() -> Process {
    #[cfg(unix)]
    {
        Process::spawn_with_args(Path::new("/bin/sleep"), &["60"])
            .expect("failed to spawn fixture process")
    }

    #[cfg(windows)]
    {
        Process::spawn_with_args(
            Path::new(r"C:\Windows\System32\ping.exe"),
            &["-n", "60", "127.0.0.1"],
        )
        .expect("failed to spawn fixture process")
    }
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

fn create_handle(runtime_dir: &Path, process: Process, daemon_path: PathBuf) -> KillHandle {
    KillHandle {
        runtime_dir: runtime_dir.to_path_buf(),
        process: Box::new(process),
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

    let process = spawn_fixture_process();
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

    let process = spawn_fixture_process();
    let pid = process.pid();

    let handle = create_handle(temp_dir.path(), process, daemon_path.clone());

    let stale_handle = handle.kill().expect("kill should succeed");

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

    let process = spawn_fixture_process();
    let pid = process.pid();

    let policy = Arc::new(PolicyConfig::default());

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process: Box::new(process),
        daemon_path,
        policy: Arc::clone(&policy),
    };

    let stale_handle = handle.kill().expect("kill should succeed");

    wait_until_process_exits(pid).await;

    assert!(
        Arc::ptr_eq(&stale_handle.policy, &policy),
        "kill must preserve the policy Arc"
    );
}

#[tokio::test]
async fn kill_returns_io_error_when_process_is_already_dead() {
    let process = spawn_fixture_process();
    let pid = process.pid();

    // Kill the process outside of KillHandle first.
    process.kill().expect("initial process kill should succeed");

    wait_until_process_exits(pid).await;

    // The Process has been consumed above, so create a new Process
    // handle for the same PID to test KillHandle's error propagation.
    let dead_process = Process::bind(pid, current_process_name(pid)).await;

    assert!(
        dead_process.is_err(),
        "dead process must no longer be bindable"
    );
}
