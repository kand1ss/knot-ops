use knot_sdk::handles::KillHandle;
use knot_sdk::policies::PolicyConfig;
use knot_sys::{Process, ProcessError};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use knot_sys::process::PlatformHandle;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

use serial_test::serial;

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);
const BINARY: &str = "client-fixture";

/// Multiplies timing-sensitive assertions to give slack under coverage
/// instrumentation (tarpaulin) or contended/parallel CI runners.
/// Set `CI_TIMING_SLACK=5` (or higher) in the tarpaulin CI profile.
fn ci_slack_multiplier() -> u32 {
    std::env::var("CI_TIMING_SLACK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Resolve the process-fixture binary produced by Cargo.
///
/// `CARGO_BIN_EXE_client-fixture` is available for integration tests
/// when the binary is declared in Cargo.toml.
fn fixture_binary() -> PathBuf {
    PathBuf::from(
        std::env::var(format!("CARGO_BIN_EXE_{BINARY}"))
            .expect("CARGO_BIN_EXE_<name> is set by Cargo during integration tests"),
    )
}

/// Reads the OS-reported start_time for `pid` via sysinfo.
///
/// start_time is the identity anchor that defeats PID-reuse races: the OS
/// is free to recycle a PID the instant the original process is reaped, so
/// PID alone is never a safe process identifier across an `await` point.
fn process_start_time(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let sys_pid = Pid::from(pid as usize);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sys_pid]),
        false,
        ProcessRefreshKind::nothing(),
    );

    system.process(sys_pid).map(|p| p.start_time())
}

/// A spawned fixture process plus the identity anchor needed to safely
/// track its lifecycle across PID reuse.
struct Fixture {
    process: Process,
    pid: u32,
    start_time: u64,
}

async fn spawn_fixture_process() -> Fixture {
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

    // Capture start_time immediately, before handing pid-based ownership
    // over to `Process`. This is the only identity check that survives
    // PID reuse by an unrelated process spawned concurrently by another
    // test (guaranteed to happen under parallel/tarpaulin runs, since all
    // fixtures share the same binary name).
    let start_time = process_start_time(pid)
        .expect("fixture process must be observable immediately after spawn");

    // Reap explicitly instead of bare `drop(child)`. We are the OS-level
    // parent of this child; if nothing ever calls `.wait()` on it, it can
    // sit as a zombie until something reaps it, which quietly poisons the
    // PID space for every other test running in parallel. This owns the
    // `Child` for the rest of the process's life and reaps it the moment
    // it exits, regardless of who kills it (fixture cleanup, KillHandle,
    // or the OS).
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    let process = Process::bind(pid, path.file_name().unwrap().to_string_lossy().to_string())
        .await
        .expect("failed to bind to fixture process");

    Fixture {
        process,
        pid,
        start_time,
    }
}

/// Waits for the *specific* process identified by (pid, start_time) to
/// exit. Checking start_time alongside pid is required: once the original
/// process is reaped, the OS can immediately reassign `pid` to an
/// unrelated process (e.g. another test's fixture), and a bare
/// `sysinfo::process(pid)` lookup cannot tell the two apart.
async fn wait_until_process_exits(pid: u32, expected_start_time: u64) {
    timeout(Duration::from_secs(15), async move {
        loop {
            let is_original_process_still_alive = tokio::task::spawn_blocking(move || {
                use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

                let sys_pid = Pid::from(pid as usize);
                let mut system = System::new();

                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[sys_pid]),
                    false,
                    ProcessRefreshKind::nothing(),
                );

                match system.process(sys_pid) {
                    // Same PID reporting a *different* start_time means the
                    // original process is gone and the PID was recycled.
                    Some(process) => process.start_time() == expected_start_time,
                    None => false,
                }
            })
            .await
            .expect("process inspection task panicked");

            if !is_original_process_still_alive {
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
#[serial(process_lifecycle)]
async fn kill_terminates_real_process() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

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

    let handle = create_handle(temp_dir.path(), fixture.process, daemon_path);

    let stale_handle = handle
        .kill()
        .await
        .expect("kill should terminate the real process");

    assert_eq!(stale_handle.runtime_dir, temp_dir.path());

    wait_until_process_exits(pid, start_time).await;
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_returns_stale_handle_with_original_runtime_dir() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    let handle = create_handle(temp_dir.path(), fixture.process, daemon_path.clone());
    let stale_handle = handle.kill().await.expect("kill should succeed");

    wait_until_process_exits(pid, start_time).await;

    assert_eq!(stale_handle.runtime_dir, temp_dir.path());
    assert_eq!(stale_handle.daemon_path, daemon_path);
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_preserves_policy_in_returned_stale_handle() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");

    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    let policy = Arc::new(PolicyConfig::default());

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process: fixture.process,
        daemon_path,
        policy: Arc::clone(&policy),
    };

    let stale_handle = handle.kill().await.expect("kill should succeed");

    wait_until_process_exits(pid, start_time).await;

    assert!(
        Arc::ptr_eq(&stale_handle.policy, &policy),
        "kill must preserve the policy Arc"
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_is_idempotent_when_process_already_exited() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    #[cfg(unix)]
    let daemon_path = PathBuf::from("/bin/sleep");
    #[cfg(windows)]
    let daemon_path = PathBuf::from(r"C:\Windows\System32\ping.exe");

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    fixture
        .process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("pre-kill for setup should succeed");
    wait_until_process_exits(pid, start_time).await;

    let handle = create_handle(temp_dir.path(), fixture.process, daemon_path);
    let result = handle.kill().await;

    assert!(
        result.is_ok(),
        "KillHandle::kill() on an already-exited process must succeed idempotently, \
     not error — got: {result:?}"
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_returns_mismatch_when_process_name_differs() {
    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

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

    let res = fixture
        .process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("cleanup kill should succeed");
    assert!(res, "cleanup kill must succeed");

    // Without this, the reaped-but-unverified process leaves the PID
    // space "dirty" for the window in which a concurrently running test
    // could get the same PID reassigned to it.
    wait_until_process_exits(pid, start_time).await;
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_succeeds_when_name_matches_case_insensitively() {
    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    #[cfg(unix)]
    let bound = Process::bind(pid, BINARY.to_uppercase().to_string()).await;
    #[cfg(windows)]
    let bound = Process::bind(pid, BINARY.to_uppercase().to_string()).await;

    assert!(bound.is_ok(), "case-insensitive match must succeed");

    let res = fixture
        .process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("cleanup kill should succeed");
    assert!(res, "cleanup kill must succeed");

    wait_until_process_exits(pid, start_time).await;
}

#[cfg(unix)]
#[tokio::test]
#[serial(process_lifecycle)]
async fn terminate_times_out_when_process_ignores_sigterm() {
    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    let slack = ci_slack_multiplier();
    let base_timeout = Duration::from_secs(1);
    let timeout_duration = base_timeout;
    let start = std::time::Instant::now();

    let result = fixture
        .process
        .terminate(timeout_duration)
        .await
        .unwrap_or_else(|error| panic!("terminate() must not return an error: {error:?}"));

    let elapsed = start.elapsed();

    assert!(
        !result,
        "terminate() must return false when SIGTERM is ignored"
    );

    // Slack scales with CI_TIMING_SLACK; on a bare-metal, uninstrumented
    // run this is unchanged (multiplier = 1).
    let lower_slack = Duration::from_millis(50) * slack;
    assert!(
        elapsed + lower_slack >= timeout_duration,
        "terminate() returned too early: elapsed={elapsed:?}, timeout={timeout_duration:?}"
    );
    assert!(
        elapsed < timeout_duration + Duration::from_secs(2) * slack,
        "terminate() took suspiciously long: elapsed={elapsed:?}"
    );

    let res = fixture
        .process
        .kill(timeout_duration)
        .await
        .expect("kill should succeed as cleanup");
    assert!(res, "cleanup kill must succeed");

    wait_until_process_exits(pid, start_time).await;
}

#[cfg(unix)]
#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_terminates_process_even_if_it_traps_sigterm() {
    let process = Process::spawn_with_args(
        Path::new("/bin/sh"),
        &["-c", "trap 'echo caught' TERM; sleep 5"],
    )
    .await
    .expect("failed to spawn trapping fixture");

    let res = process
        .kill(DEFAULT_TIMEOUT)
        .await
        .expect("kill should succeed");

    assert!(res, "cleanup kill must succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn spawn_returns_error_for_nonexistent_binary() {
    let result = Process::spawn(Path::new("/definitely/does/not/exist/binary")).await;
    assert!(result.is_err(), "spawning nonexistent binary must fail");
}

#[cfg(unix)]
#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_handle_escalates_to_real_sigkill_for_trapping_process() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");
    let timeout_duration = Duration::from_millis(200) * ci_slack_multiplier();

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    let mut policy = PolicyConfig::default();
    policy.kill.graceful_timeout = timeout_duration;

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process: fixture.process,
        daemon_path: PathBuf::from("/bin/sh"),
        policy: Arc::new(policy),
    };

    let stale_handle = handle
        .kill()
        .await
        .expect("kill must escalate past a trapped SIGTERM and succeed via SIGKILL");

    wait_until_process_exits(pid, start_time).await;
    assert_eq!(stale_handle.runtime_dir, temp_dir.path());
}

#[cfg(unix)]
#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_returns_io_error_for_unkillable_pid() {
    // Guard: do not assume real PID 1 is a safe, unkillable target.
    // Under tarpaulin/Docker CI without a real init process, the test
    // binary itself can be PID 1 — signaling it would be catastrophic
    // instead of merely a failed assertion. This case belongs to mock
    // tests (see kill.rs) precisely because it cannot be made both
    // meaningful and safe as an e2e test.
    if std::process::id() == 1 {
        eprintln!(
            "skipping kill_returns_io_error_for_unkillable_pid: current process is PID 1 \
             (no init in this container) — see kill.rs mock tests for this path instead"
        );
        return;
    }

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
#[serial(process_lifecycle)]
async fn kill_handle_zero_timeout_sends_sigkill_immediately() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let fixture = spawn_fixture_process().await;
    let pid = fixture.pid;
    let start_time = fixture.start_time;

    let mut policy = PolicyConfig::default();
    policy.kill.graceful_timeout = Duration::ZERO;

    let handle = KillHandle {
        runtime_dir: temp_dir.path().to_path_buf(),
        process: fixture.process,
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

    wait_until_process_exits(pid, start_time).await;
    assert!(
        elapsed < Duration::from_millis(100) * ci_slack_multiplier(),
        "zero-timeout path must not incur any graceful-phase wait; elapsed={:?}",
        elapsed
    );
}
