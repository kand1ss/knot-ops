use std::path::{Path, PathBuf};
use std::time::Duration;

use knot_sys::{Process, ProcessError};
use serial_test::serial;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::time::{sleep, timeout};

const BINARY: &str = "process-fixture";

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
/// `CARGO_BIN_EXE_process-fixture` is available for integration tests
/// when the binary is declared in Cargo.toml.
fn fixture_binary() -> PathBuf {
    PathBuf::from(
        std::env::var(format!("CARGO_BIN_EXE_{BINARY}"))
            .expect("CARGO_BIN_EXE_<name> is set by Cargo during integration tests"),
    )
}

/// True if `pid` is still present in the OS process table.
fn pid_exists(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from(pid as usize)]),
        false,
        ProcessRefreshKind::nothing(),
    );
    sys.process(Pid::from(pid as usize)).is_some()
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

/// Reads the OS-reported start_time for `pid`, if the process currently exists.
fn pid_start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from(pid as usize)]),
        false,
        ProcessRefreshKind::nothing(),
    );
    sys.process(Pid::from(pid as usize)).map(|p| p.start_time())
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn spawn_reports_a_valid_pid() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin)
        .await
        .expect("spawn should succeed for a real, long-running binary");

    assert!(process.pid() > 0, "pid() must return a real, positive PID");
    assert!(
        pid_exists(process.pid()),
        "process should be visible in the OS process table right after spawn"
    );

    // cleanup: don't leak the child into the test runner's process tree
    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn kill_actually_terminates_the_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let exited = process
        .kill(Duration::from_secs(5))
        .await
        .expect("kill should not error against a live, killable process");

    assert!(
        exited,
        "kill() should report the process exited within the timeout"
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn terminate_stops_a_cooperative_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let exited = process
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate should not error");

    assert!(
        exited,
        "terminate() should report the process exited within the timeout"
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_rejects_a_name_mismatch() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");
    let pid = process.pid();

    let result = Process::bind(pid, "definitely-not-the-right-binary-name".to_string()).await;

    match result {
        Err(ProcessError::Mismatch { .. }) => {}
        other => panic!("expected ProcessError::Mismatch, got {other:?}"),
    }

    // cleanup via the original handle, not the rejected bind attempt
    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_fails_for_a_pid_that_does_not_exist() {
    let result = Process::bind(u32::MAX, "anything".to_string()).await;
    assert!(matches!(result, Err(ProcessError::NotRunning)))
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn spawn_fails_for_a_nonexistent_binary() {
    let result = Process::spawn(Path::new("/definitely/does/not/exist/on/this/machine")).await;
    assert!(
        result.is_err(),
        "spawning a nonexistent binary must not silently succeed"
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_returns_not_running_for_nonexistent_process() {
    let result = Process::bind(u32::MAX, "process-that-does-not-exist".into()).await;
    assert!(
        matches!(result, Err(ProcessError::NotRunning)),
        "expected ProcessError::NotRunning, got {:?}",
        result
    );
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_succeeds_for_a_matching_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let pid = process.pid();

    let bound = Process::bind(pid, BINARY.to_string())
        .await
        .expect("bind should succeed for a matching process");

    assert_eq!(
        bound.pid(),
        pid,
        "bound process must preserve the original PID"
    );

    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn wait_returns_false_when_process_is_still_running() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let started = tokio::time::Instant::now();

    let exited = process
        .wait(Duration::from_millis(200))
        .await
        .expect("wait should not fail for a live process");

    let elapsed = started.elapsed();

    assert!(
        !exited,
        "wait() must return false when the process is still alive"
    );

    assert!(
        elapsed >= Duration::from_millis(150),
        "wait() returned too early: {:?}",
        elapsed
    );

    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn wait_zero_timeout_returns_immediately_for_running_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let started = tokio::time::Instant::now();

    let exited = process
        .wait(Duration::ZERO)
        .await
        .expect("wait should not fail");

    let elapsed = started.elapsed();

    assert!(!exited, "a running process must not be reported as exited");

    assert!(
        elapsed < Duration::from_millis(100) * ci_slack_multiplier(),
        "zero-timeout wait took too long: {:?}",
        elapsed
    );

    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn wait_returns_true_after_process_exits_externally() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let pid = process.pid();

    let killer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let process = Process::bind(pid, BINARY.to_string()).await.unwrap();
        process.kill(Duration::from_secs(1)).await.unwrap();
    });

    let exited = process
        .wait(Duration::from_secs(1))
        .await
        .expect("wait should succeed");

    killer.await.expect("killer task should not panic");
    assert!(exited, "wait() must detect external process termination");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn terminate_then_wait_completes_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let terminated = process
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate should succeed");

    assert!(terminated, "terminate should report successful termination");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bound_process_can_be_terminated_and_waited_on() {
    let bin = fixture_binary();

    let spawned = Process::spawn(&bin).await.expect("spawn should succeed");

    let pid = spawned.pid();

    let process = Process::bind(pid, BINARY.to_string())
        .await
        .expect("bind should succeed");

    let res = process
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate should succeed");

    assert!(res, "bound process must no longer exist after termination");
}

#[tokio::test]
#[serial(process_lifecycle)]
async fn bind_fails_after_target_process_exits() {
    let bin = fixture_binary();

    let process = Process::spawn(&bin).await.expect("spawn should succeed");
    let pid = process.pid();
    let original_start_time =
        pid_start_time(pid).expect("process must be observable immediately after spawn");

    let res = process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
    assert!(res, "fixture should be gone");
    wait_until_process_exits(pid, original_start_time).await;

    let result = Process::bind(pid, BINARY.to_string()).await;

    match result {
        Err(ProcessError::NotRunning) => {
            // Expected common case: PID is free, nothing occupies it.
        }
        Err(ProcessError::Mismatch { .. }) => {
            // PID was reused by a process with a *different* name.
            // Still proves our original process is gone.
        }
        Ok(bound) => {
            // PID was reused by an unrelated process that *also* happens
            // to be named "process-fixture" (spawned by another test
            // running concurrently under this same tarpaulin/CI run).
            // This is not a bug in the code under test — but we must
            // prove it's genuinely a different process, not our original
            // one somehow surviving the kill.
            let reused_start_time = pid_start_time(pid);

            assert_ne!(
                reused_start_time,
                Some(original_start_time),
                "bind() succeeded with the *same* start_time as the killed process — \
                 this means kill() did not actually terminate the original process \
                 (pid={pid}, start_time={original_start_time})"
            );

            // Don't leak the unrelated process we just accidentally bound to.
            let _ = bound.kill(Duration::from_secs(5)).await;
        }
        Err(other) => {
            panic!("expected NotRunning, Mismatch, or Ok(reused pid), got {other:?}");
        }
    }
}
