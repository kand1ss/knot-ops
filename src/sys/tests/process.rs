use std::path::{Path, PathBuf};
use std::time::Duration;

use knot_sys::{Process, ProcessError};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const BINARY: &str = "process-fixture";

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

/// Polls until `pid` disappears from the process table or `timeout` elapses.
/// Used only for test cleanup/assertions, never to drive library logic.
async fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if !pid_exists(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    !pid_exists(pid)
}

#[tokio::test]
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
async fn kill_actually_terminates_the_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");
    let pid = process.pid();

    let exited = process
        .kill(Duration::from_secs(5))
        .await
        .expect("kill should not error against a live, killable process");

    assert!(
        exited,
        "kill() should report the process exited within the timeout"
    );
    assert!(
        wait_until_gone(pid, Duration::from_secs(2)).await,
        "process must actually be gone from the OS process table after kill()"
    );
}

#[tokio::test]
async fn terminate_stops_a_cooperative_process() {
    let bin = fixture_binary();
    let process = Process::spawn(&bin).await.expect("spawn should succeed");
    let pid = process.pid();

    let exited = process
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate should not error");

    assert!(
        exited,
        "terminate() should report the process exited within the timeout"
    );
    assert!(
        wait_until_gone(pid, Duration::from_secs(2)).await,
        "process must actually be gone after terminate()"
    );
}

#[tokio::test]
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
async fn bind_fails_for_a_pid_that_does_not_exist() {
    let result = Process::bind(u32::MAX, "anything".to_string()).await;
    assert!(matches!(result, Err(ProcessError::NotRunning)))
}

#[tokio::test]
async fn spawn_fails_for_a_nonexistent_binary() {
    let result = Process::spawn(Path::new("/definitely/does/not/exist/on/this/machine")).await;
    assert!(
        result.is_err(),
        "spawning a nonexistent binary must not silently succeed"
    );
}

#[tokio::test]
async fn bind_returns_not_running_for_nonexistent_process() {
    let result = Process::bind(u32::MAX, "process-that-does-not-exist".into()).await;
    assert!(
        matches!(result, Err(ProcessError::NotRunning)),
        "expected ProcessError::NotRunning, got {:?}",
        result
    );
}

#[tokio::test]
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
        elapsed < Duration::from_millis(100),
        "zero-timeout wait took too long: {:?}",
        elapsed
    );

    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");
}

#[tokio::test]
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

    assert!(
        wait_until_gone(pid, Duration::from_secs(2)).await,
        "process must actually be gone"
    );
}

#[tokio::test]
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
async fn bound_process_can_be_terminated_and_waited_on() {
    let bin = fixture_binary();

    let spawned = Process::spawn(&bin).await.expect("spawn should succeed");

    let pid = spawned.pid();

    let process = Process::bind(pid, BINARY.to_string())
        .await
        .expect("bind should succeed");

    process
        .terminate(Duration::from_secs(5))
        .await
        .expect("terminate should succeed");

    assert!(
        !pid_exists(pid),
        "bound process must no longer exist after termination"
    );
}

#[tokio::test]
async fn bind_fails_after_target_process_exits() {
    let bin = fixture_binary();

    let process = Process::spawn(&bin).await.expect("spawn should succeed");

    let pid = process.pid();

    process
        .kill(Duration::from_secs(5))
        .await
        .expect("cleanup kill should succeed");

    assert!(
        wait_until_gone(pid, Duration::from_secs(2)).await,
        "fixture should be gone"
    );

    let result = Process::bind(pid, BINARY.to_string()).await;

    assert!(
        matches!(result, Err(ProcessError::NotRunning)),
        "expected NotRunning, got {result:?}"
    );
}
