package processlock

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

const (
	helperEnv      = "PROCESSLOCK_HELPER"
	lockPathEnv    = "PROCESSLOCK_PATH"
	readyPathEnv   = "PROCESSLOCK_READY"
	releasePathEnv = "PROCESSLOCK_RELEASE"
)

// TestAcquire_EmptyPath verifies that an empty path is rejected.
func TestAcquire_EmptyPath(t *testing.T) {
	_, err := Acquire("")
	if err == nil {
		t.Fatal("expected error for empty path")
	}
}

// TestAcquire_InvalidPath verifies that filesystem errors are propagated.
func TestAcquire_InvalidPath(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing", "lock")

	_, err := Acquire(path)
	if err == nil {
		t.Fatal("expected error for invalid path")
	}
}

// TestAcquire_CreatesLockFile verifies that Acquire creates the lock file.
func TestAcquire_CreatesLockFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.lock")

	lock, err := Acquire(path)
	if err != nil {
		t.Fatalf("Acquire() error = %v", err)
	}
	defer lock.Close()

	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("lock file was not created: %v", err)
	}

	if !info.Mode().IsRegular() {
		t.Fatalf("expected regular file, got %v", info.Mode())
	}
}

// TestAcquire_LockReleasedAfterClose verifies that another lock
// can be acquired after Close.
func TestAcquire_LockReleasedAfterClose(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.lock")

	first, err := Acquire(path)
	if err != nil {
		t.Fatalf("first Acquire() error = %v", err)
	}

	if err := first.Close(); err != nil {
		t.Fatalf("Close() error = %v", err)
	}

	second, err := Acquire(path)
	if err != nil {
		t.Fatalf("second Acquire() error = %v", err)
	}
	defer second.Close()
}

// TestAcquire_RejectsSecondLock verifies exclusivity between
// independently opened handles in the same process.
func TestAcquire_RejectsSecondLock(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.lock")

	first, err := Acquire(path)
	if err != nil {
		t.Fatalf("first Acquire() error = %v", err)
	}
	defer first.Close()

	second, err := Acquire(path)
	if !errors.Is(err, ErrAlreadyLocked) {
		if err == nil {
			_ = second.Close()
		}
		t.Fatalf("expected ErrAlreadyLocked, got %v", err)
	}
}

// TestFileLock_CloseIsIdempotent verifies that repeated Close calls
// do not panic or cause additional resource operations.
func TestFileLock_CloseIsIdempotent(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.lock")

	lock, err := Acquire(path)
	if err != nil {
		t.Fatalf("Acquire() error = %v", err)
	}

	if err := lock.Close(); err != nil {
		t.Fatalf("first Close() error = %v", err)
	}

	if err := lock.Close(); err != nil {
		t.Fatalf("second Close() error = %v", err)
	}
}

// TestFileLock_ConcurrentClose verifies that concurrent Close calls
// are safe.
func TestFileLock_ConcurrentClose(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.lock")

	lock, err := Acquire(path)
	if err != nil {
		t.Fatalf("Acquire() error = %v", err)
	}

	const workers = 32

	var wg sync.WaitGroup
	errs := make(chan error, workers)

	for i := 0; i < workers; i++ {
		wg.Add(1)

		go func() {
			defer wg.Done()
			errs <- lock.Close()
		}()
	}

	wg.Wait()
	close(errs)

	for err := range errs {
		if err != nil {
			t.Errorf("concurrent Close() error = %v", err)
		}
	}

	// Verify that the underlying lock was actually released.
	reopened, err := Acquire(path)
	if err != nil {
		t.Fatalf("Acquire() after concurrent Close() error = %v", err)
	}
	defer reopened.Close()
}

// TestFileLock_ProcessExclusivity verifies that a separate process
// cannot acquire a lock held by the parent process.
func TestFileLock_ProcessExclusivity(t *testing.T) {
	if os.Getenv(helperEnv) == "1" {
		return
	}

	dir := t.TempDir()
	lockPath := filepath.Join(dir, "daemon.lock")
	readyPath := filepath.Join(dir, "ready")
	releasePath := filepath.Join(dir, "release")

	cmd := exec.Command(os.Args[0], "-test.run=^TestFileLockHelperProcess$")
	cmd.Env = append(os.Environ(),
		helperEnv+"=1",
		lockPathEnv+"="+lockPath,
		readyPathEnv+"="+readyPath,
		releasePathEnv+"="+releasePath,
	)

	if err := cmd.Start(); err != nil {
		t.Fatalf("starting helper process: %v", err)
	}

	waited := false
	t.Cleanup(func() {
		// Ensure the helper cannot remain blocked after a test failure.
		_ = os.WriteFile(releasePath, nil, 0600)

		if !waited && cmd.Process != nil {
			_ = cmd.Process.Kill()
			_ = cmd.Wait()
		}
	})

	if err := waitForFile(readyPath, 5*time.Second); err != nil {
		t.Fatalf("helper did not acquire lock: %v", err)
	}

	// The helper process currently holds the lock.
	lock, err := Acquire(lockPath)
	if !errors.Is(err, ErrAlreadyLocked) {
		if err == nil {
			_ = lock.Close()
		}
		t.Fatalf("expected ErrAlreadyLocked, got %v", err)
	}

	// Allow the helper to release the lock.
	if err := os.WriteFile(releasePath, nil, 0600); err != nil {
		t.Fatalf("writing release signal: %v", err)
	}

	if err := cmd.Wait(); err != nil {
		waited = true
		t.Fatalf("helper process failed: %v", err)
	}
	waited = true

	// The lock must be available after the helper exits.
	lock, err = Acquire(lockPath)
	if err != nil {
		t.Fatalf("Acquire() after helper exit: %v", err)
	}
	defer lock.Close()
}

// TestFileLockHelperProcess is executed as a subprocess.
func TestFileLockHelperProcess(t *testing.T) {
	if os.Getenv(helperEnv) != "1" {
		return
	}

	lockPath := os.Getenv(lockPathEnv)
	readyPath := os.Getenv(readyPathEnv)
	releasePath := os.Getenv(releasePathEnv)

	if lockPath == "" || readyPath == "" || releasePath == "" {
		t.Fatal("missing helper environment variables")
	}

	lock, err := Acquire(lockPath)
	if err != nil {
		t.Fatalf("helper Acquire() error = %v", err)
	}
	defer lock.Close()

	if err := os.WriteFile(readyPath, nil, 0600); err != nil {
		t.Fatalf("writing ready signal: %v", err)
	}

	if err := waitForFile(releasePath, 10*time.Second); err != nil {
		t.Fatal(err)
	}
}

func waitForFile(path string, timeout time.Duration) error {
	deadline := time.Now().Add(timeout)

	for time.Now().Before(deadline) {
		_, err := os.Stat(path)

		if err == nil {
			return nil
		}

		if !os.IsNotExist(err) {
			return err
		}

		time.Sleep(10 * time.Millisecond)
	}

	return errors.New("timed out waiting for file: " + path)
}
