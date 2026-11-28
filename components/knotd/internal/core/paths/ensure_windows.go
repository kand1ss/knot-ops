//go:build windows

package paths

import (
	"fmt"
	"os"
)

// %LOCALAPPDATA% is already per-user ACL'd; MkdirAll is sufficient.
func EnsureRuntimeDir(dir string) error {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("create runtime dir: %w", err)
	}
	return nil
}
