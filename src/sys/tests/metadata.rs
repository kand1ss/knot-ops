use knot_sys::metadata::ProcessMetadata;
use std::io;
use std::path::PathBuf;
use std::process;
use std::process::{Child, Command};
use std::time::Duration;

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

struct Fixture {
    child: Child,
}

impl Fixture {
    fn spawn() -> Self {
        let child = Command::new(fixture_binary())
            .spawn()
            .expect("failed to spawn process fixture");

        Self { child }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn extracts_current_process_metadata() {
    let pid = process::id();

    let metadata =
        ProcessMetadata::extract(pid).expect("failed to extract current process metadata");

    assert_eq!(metadata.pid, pid);
    assert!(!metadata.name.is_empty());
    assert!(metadata.start_time > 0);
}

#[test]
fn returns_not_found_for_nonexistent_process() {
    // PID 0 has special OS semantics, so don't use it.
    // Use a value very unlikely to be allocated.
    let pid = u32::MAX;

    let result = ProcessMetadata::extract(pid);

    let error = result.expect_err("expected process lookup to fail");

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn extracts_stable_metadata_for_same_process() {
    let pid = process::id();

    let first = ProcessMetadata::extract(pid).expect("failed to extract process metadata");

    let second = ProcessMetadata::extract(pid).expect("failed to extract process metadata");

    assert_eq!(first, second);
}

#[test]
fn extracts_metadata_from_fixture_process() {
    let fixture = Fixture::spawn();

    let metadata =
        ProcessMetadata::extract(fixture.pid()).expect("failed to extract fixture metadata");

    assert_eq!(metadata.pid, fixture.pid());
    assert!(!metadata.name.is_empty());
    assert!(metadata.start_time > 0);
}

#[test]
fn extracts_stable_metadata_from_fixture_process() {
    let fixture = Fixture::spawn();

    let first =
        ProcessMetadata::extract(fixture.pid()).expect("failed to extract fixture metadata");

    std::thread::sleep(Duration::from_millis(50));

    let second =
        ProcessMetadata::extract(fixture.pid()).expect("failed to extract fixture metadata");

    assert_eq!(first, second);
}

#[test]
fn extracted_pid_matches_requested_pid() {
    let pid = process::id();

    let metadata = ProcessMetadata::extract(pid).expect("failed to extract process metadata");

    assert_eq!(metadata.pid, pid);
}

#[test]
fn extracts_process_identity_metadata() {
    let pid = process::id();

    let metadata = ProcessMetadata::extract(pid).expect("failed to extract process metadata");

    assert_eq!(metadata.pid, pid);
    assert!(!metadata.name.is_empty());
    assert_ne!(
        metadata.start_time, 0,
        "process start time must be available"
    );
}

#[test]
fn metadata_becomes_unavailable_after_fixture_exits() {
    let mut fixture = Fixture::spawn();
    let pid = fixture.pid();

    let metadata = ProcessMetadata::extract(pid).expect("fixture should exist");

    assert_eq!(metadata.pid, pid);
    assert!(metadata.start_time > 0);

    fixture.child.kill().expect("failed to kill fixture");
    fixture.child.wait().expect("failed to reap fixture");

    let result = ProcessMetadata::extract(pid);

    assert!(
        result.is_err(),
        "metadata lookup should fail after fixture exits"
    );
}
