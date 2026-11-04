//go:build !windows

package paths

import (
	"fmt"
	"os"
	"syscall"
)

// EnsureRuntimeDir creates dir with 0700 and verifies ownership and mode.
// Required because the /tmp fallback is predictable and could be
// pre-created by another user.
func EnsureRuntimeDir(dir string) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("create runtime dir: %w", err)
	}
	// Lstat: do not follow a symlink planted at the path.
	st, err := os.Lstat(dir)
	if err != nil {
		return fmt.Errorf("stat runtime dir: %w", err)
	}
	if !st.IsDir() {
		return fmt.Errorf("runtime path %q is not a directory (symlink?)", dir)
	}
	sys, ok := st.Sys().(*syscall.Stat_t)
	if !ok || int(sys.Uid) != os.Getuid() {
		return fmt.Errorf("runtime dir %q is not owned by current user", dir)
	}
	if st.Mode().Perm() != 0o700 {
		if err := os.Chmod(dir, 0o700); err != nil {
			return fmt.Errorf("fix runtime dir mode: %w", err)
		}
	}
	return nil
}
