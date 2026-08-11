use knot_client::errors::{ClientError, DaemonLifecycleError};
use knot_client::handles::OfflineHandle;
use knot_client::policies::PolicyConfig;
use knot_core::consts::KNOT_SOCKET_FILE;

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

/// Resolve the pre-built process fixture.
///
/// The fixture is compiled by Cargo and therefore does not need to be
/// created inside TempDir at runtime. This avoids Unix `ETXTBSY`
/// ("Text file busy") races with the executable.
fn fixture_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_process-fixture"))
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

#[tokio::test]
async fn test_launch_timeout_when_socket_never_appears() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    let handle = create_handle(&temp_dir, fixture_binary());

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
async fn test_launch_does_not_treat_regular_file_as_socket() {
    let temp_dir = TempDir::new().expect("failed to create temp directory");

    let socket_path = temp_dir.path().join(KNOT_SOCKET_FILE);

    std::fs::write(&socket_path, "this is not a socket")
        .expect("failed to create fake socket file");

    let handle = create_handle(&temp_dir, fixture_binary());

    let result = handle.launch().await;

    assert!(
        result.is_err(),
        "launch must not accept a regular file as a valid socket"
    );
}
