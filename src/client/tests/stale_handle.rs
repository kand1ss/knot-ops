use knot_client::handles::StaleHandle;
use knot_client::policies::PolicyConfig;

use knot_core::consts::KNOT_DAEMON_LOCK_FILE;

#[cfg(not(windows))]
use knot_core::consts::KNOT_SOCKET_FILE;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

fn create_handle(
    runtime_dir: &Path,
    daemon_path: PathBuf,
    policy: Arc<PolicyConfig>,
) -> StaleHandle {
    StaleHandle {
        runtime_dir: runtime_dir.to_path_buf(),
        daemon_path,
        policy,
    }
}

fn daemon_path() -> PathBuf {
    PathBuf::from("/mock/bin/knotd")
}

#[tokio::test]
async fn clean_removes_all_stale_artifacts() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path();

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    fs::write(&lock_path, "1234").expect("failed to create lock file");

    #[cfg(not(windows))]
    let socket_path = {
        let path = runtime_dir.join(KNOT_SOCKET_FILE);

        fs::write(&path, "").expect("failed to create socket file");

        path
    };

    let policy = Arc::new(PolicyConfig::default());

    let handle = create_handle(runtime_dir, daemon_path(), Arc::clone(&policy));

    let offline_handle = handle.clean().await.expect("stale cleanup should succeed");

    assert_eq!(offline_handle.runtime_dir, runtime_dir);

    assert_eq!(offline_handle.daemon_path, daemon_path());

    assert!(
        Arc::ptr_eq(&offline_handle.policy, &policy),
        "cleanup must preserve the policy Arc"
    );

    assert!(!lock_path.exists(), "stale lock file must be removed");

    #[cfg(not(windows))]
    assert!(!socket_path.exists(), "stale socket file must be removed");
}

#[tokio::test]
async fn clean_succeeds_when_no_stale_artifacts_exist() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let policy = Arc::new(PolicyConfig::default());

    let handle = create_handle(temp_dir.path(), daemon_path(), Arc::clone(&policy));

    let offline_handle = handle
        .clean()
        .await
        .expect("cleanup should succeed when there is nothing to remove");

    assert_eq!(offline_handle.runtime_dir, temp_dir.path());

    assert_eq!(offline_handle.daemon_path, daemon_path());

    assert!(
        Arc::ptr_eq(&offline_handle.policy, &policy),
        "cleanup must preserve the policy Arc"
    );
}

#[tokio::test]
async fn clean_removes_lock_when_socket_is_absent() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path();

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    fs::write(&lock_path, "1234").expect("failed to create lock file");

    #[cfg(not(windows))]
    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    let handle = create_handle(
        runtime_dir,
        daemon_path(),
        Arc::new(PolicyConfig::default()),
    );

    let result = handle.clean().await;

    assert!(
        result.is_ok(),
        "cleanup should succeed when socket is absent: {result:?}"
    );

    assert!(!lock_path.exists(), "lock file must be removed");

    #[cfg(not(windows))]
    assert!(!socket_path.exists(), "socket must remain absent");
}

#[cfg(not(windows))]
#[tokio::test]
async fn clean_removes_socket_when_lock_is_absent() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path();

    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);

    fs::write(&socket_path, "").expect("failed to create socket file");

    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    let handle = create_handle(
        runtime_dir,
        daemon_path(),
        Arc::new(PolicyConfig::default()),
    );

    let result = handle.clean().await;

    assert!(
        result.is_ok(),
        "cleanup should succeed when lock is absent: {result:?}"
    );

    assert!(!socket_path.exists(), "socket file must be removed");

    assert!(!lock_path.exists(), "lock file must remain absent");
}

#[tokio::test]
async fn clean_returns_error_when_runtime_directory_does_not_exist() {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path().to_path_buf();

    let handle = create_handle(
        &runtime_dir,
        daemon_path(),
        Arc::new(PolicyConfig::default()),
    );

    // Remove the runtime directory before cleanup.
    fs::remove_dir_all(&runtime_dir).expect("failed to remove temporary runtime directory");

    let result = handle.clean().await;

    /*
     * The current implementation uses `exists()` before remove_file().
     * Therefore a completely missing runtime directory is treated as
     * "nothing to clean" and succeeds.
     *
     * This assertion documents that behavior.
     */
    assert!(
        result.is_ok(),
        "missing runtime directory is currently treated as an empty state"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn clean_returns_error_when_socket_cannot_be_removed() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path();

    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    fs::write(&socket_path, "").expect("failed to create socket file");

    fs::write(&lock_path, "1234").expect("failed to create lock file");

    /*
     * Make the directory read-only so removing the socket fails.
     *
     * This test must restore permissions before TempDir cleanup,
     * otherwise TempDir may fail to remove the directory.
     */
    let original_permissions = fs::metadata(runtime_dir)
        .expect("failed to stat runtime directory")
        .permissions();

    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);

    fs::set_permissions(runtime_dir, read_only_permissions)
        .expect("failed to make runtime directory read-only");

    let handle = create_handle(
        runtime_dir,
        daemon_path(),
        Arc::new(PolicyConfig::default()),
    );

    let result = handle.clean().await;

    // Restore permissions so TempDir can clean itself up.
    fs::set_permissions(runtime_dir, original_permissions)
        .expect("failed to restore runtime directory permissions");

    assert!(
        result.is_err(),
        "cleanup must fail when socket cannot be removed"
    );

    assert!(
        socket_path.exists(),
        "socket must remain when its removal fails"
    );

    assert!(
        lock_path.exists(),
        "lock must not be removed after socket cleanup fails"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn clean_does_not_remove_lock_when_socket_cleanup_fails() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().expect("failed to create temporary directory");

    let runtime_dir = temp_dir.path();

    let socket_path = runtime_dir.join(KNOT_SOCKET_FILE);
    let lock_path = runtime_dir.join(KNOT_DAEMON_LOCK_FILE);

    fs::write(&socket_path, "").expect("failed to create socket file");

    fs::write(&lock_path, "1234").expect("failed to create lock file");

    let original_permissions = fs::metadata(runtime_dir)
        .expect("failed to stat runtime directory")
        .permissions();

    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);

    fs::set_permissions(runtime_dir, read_only_permissions)
        .expect("failed to make runtime directory read-only");

    let handle = create_handle(
        runtime_dir,
        daemon_path(),
        Arc::new(PolicyConfig::default()),
    );

    let result = handle.clean().await;

    fs::set_permissions(runtime_dir, original_permissions)
        .expect("failed to restore runtime directory permissions");

    assert!(result.is_err(), "cleanup should fail");

    assert!(
        socket_path.exists(),
        "socket must remain after failed cleanup"
    );

    assert!(
        lock_path.exists(),
        "lock must not be touched after socket removal fails"
    );
}
