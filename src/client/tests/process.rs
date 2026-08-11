use std::io;
use std::path::PathBuf;
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate,  ProcessStatus, System};
use tokio::time::{sleep, timeout};

use knot_client::process::{Process, ProcessControl, ProcessError};

/// Returns the PID of the current test process.
fn current_pid() -> u32 {
    std::process::id()
}

/// Returns the executable name of the current process according to sysinfo.
fn current_process_name() -> String {
    let pid = Pid::from(std::process::id() as usize);

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing(),
    );

    system
        .process(pid)
        .expect("current process must be visible in sysinfo")
        .name()
        .to_string_lossy()
        .into_owned()
}

/// Returns a PID which is very unlikely to exist.
///
/// We deliberately use a value near the upper end of u32 rather than
/// assuming that PID 0 is invalid, because PID 0 has platform-specific
/// semantics.
fn nonexistent_pid() -> u32 {
    u32::MAX
}

/// Resolve the process-fixture binary produced by Cargo.
///
/// `CARGO_BIN_EXE_process-fixture` is available for integration tests
/// when the binary is declared in Cargo.toml.
fn fixture_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_process-fixture"))
}

async fn wait_until_not_running(pid: u32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let sys_pid = Pid::from(pid as usize);

            let mut system = System::new();

            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[sys_pid]),
                false,
                ProcessRefreshKind::nothing(),
            );

            match system.process(sys_pid) {
                None => return,

                Some(process) if process.status() == ProcessStatus::Zombie => {
                    // On Unix, SIGKILL terminates the process immediately,
                    // but a child process remains visible as a zombie until
                    // its parent reaps it with waitpid().
                    //
                    // For the purpose of this test, a zombie is already
                    // terminated.
                    return;
                }

                Some(_) => {
                    sleep(Duration::from_millis(25)).await;
                }
            }
        }
    })
        .await
        .expect("process did not terminate within timeout");
}

struct ProcessGuard(Process);

impl ProcessGuard {
    fn spawn(binary: &std::path::Path) -> io::Result<Self> {
        Ok(Self(Process::spawn(binary)?))
    }

    fn pid(&self) -> u32 {
        self.0.pid()
    }

    fn process(&self) -> &Process {
        &self.0
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[tokio::test]
async fn bind_succeeds_for_running_process_with_matching_name() {
    let pid = current_pid();
    let name = current_process_name();

    let process = Process::bind(pid, name)
        .await
        .expect("bind should succeed for the current process");

    assert_eq!(process.pid(), pid);
}

#[tokio::test]
async fn bind_matches_process_name_case_insensitively() {
    let pid = current_pid();
    let actual_name = current_process_name();

    let expected_name = if actual_name.chars().any(|c| c.is_ascii_lowercase()) {
        actual_name.to_ascii_uppercase()
    } else {
        actual_name.to_ascii_lowercase()
    };

    let process = Process::bind(pid, expected_name)
        .await
        .expect("process name comparison should be case-insensitive");

    assert_eq!(process.pid(), pid);
}

#[tokio::test]
async fn bind_returns_mismatch_for_running_process_with_wrong_name() {
    let pid = current_pid();

    let result = Process::bind(pid, "definitely-not-the-current-process".into()).await;

    match result {
        Err(ProcessError::Mismatch {
            actual: actual_name,
            ..
        }) => {
            assert!(!actual_name.is_empty());
            assert_eq!(actual_name, current_process_name());
        }

        other => panic!("expected ProcessError::Mismatch, got {:?}", other),
    }
}

#[tokio::test]
async fn bind_returns_not_running_for_nonexistent_process() {
    let result = Process::bind(nonexistent_pid(), "process-that-does-not-exist".into()).await;

    assert!(
        matches!(result, Err(ProcessError::NotRunning)),
        "expected ProcessError::NotRunning, got {:?}",
        result
    );
}

#[tokio::test]
async fn spawn_starts_process() {
    let binary = fixture_binary();

    let process = ProcessGuard::spawn(&binary).expect("fixture process should spawn");

    let pid = process.pid();

    assert!(pid > 0);

    // Verify that the process actually exists.
    let sys_pid = Pid::from(pid as usize);

    let mut system = System::new();

    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sys_pid]),
        false,
        ProcessRefreshKind::nothing(),
    );

    assert!(
        system.process(sys_pid).is_some(),
        "spawn returned PID {pid}, but process is not visible"
    );

    process
        .process()
        .kill()
        .expect("fixture process should be killable");
}

#[tokio::test]
async fn spawned_process_can_be_bound() {
    let binary = fixture_binary();

    let spawned = ProcessGuard::spawn(&binary).expect("fixture process should spawn");

    let pid = spawned.pid();

    let expected_name = {
        let sys_pid = Pid::from(pid as usize);
        let mut system = System::new();

        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sys_pid]),
            false,
            ProcessRefreshKind::nothing(),
        );

        system
            .process(sys_pid)
            .expect("spawned process should exist")
            .name()
            .to_string_lossy()
            .into_owned()
    };

    let bound = Process::bind(pid, expected_name)
        .await
        .expect("bind should succeed for spawned process");

    assert_eq!(bound.pid(), pid);

    bound.kill().expect("spawned process should be killable");

    wait_until_not_running(pid).await;
}

#[tokio::test]
async fn kill_terminates_spawned_process() {
    let process = ProcessGuard::spawn(&fixture_binary()).expect("fixture process should spawn");

    let pid = process.pid();

    process
        .process()
        .kill()
        .expect("kill should terminate process");

    wait_until_not_running(pid).await;
}

#[tokio::test]
async fn spawn_returns_error_for_missing_binary() {
    let path =
        std::env::temp_dir().join(format!("process-test-nonexistent-{}", std::process::id()));

    assert!(!path.exists());

    let result = Process::spawn(&path);

    assert!(result.is_err(), "spawning a nonexistent binary must fail");
}
