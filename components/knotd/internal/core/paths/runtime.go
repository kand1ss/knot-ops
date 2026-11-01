package paths

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
)

const (
	envRuntimeDir = "KNOT_RUNTIME_DIR"
	appDirName    = "knot"
	socketFile    = "knot.sock" // keep in sync with knot-core consts
	// Conservative sockaddr_un budget; matches resolve_socket_name (100).
	maxSocketPath = 100
)

var ErrRuntimeDir = errors.New("cannot resolve runtime dir")

// RuntimeDir returns the per-user runtime directory. It does NOT create it.
func RuntimeDir() (string, error) {
	dir, err := resolveRuntimeDir()
	if err != nil {
		return "", err
	}
	if runtime.GOOS != "windows" {
		if n := len(filepath.Join(dir, socketFile)); n > maxSocketPath {
			return "", fmt.Errorf("%w: socket path too long (%d > %d): %s",
				ErrRuntimeDir, n, maxSocketPath, dir)
		}
	}
	return dir, nil
}

func resolveRuntimeDir() (string, error) {
	if v := os.Getenv(envRuntimeDir); v != "" {
		if !filepath.IsAbs(v) {
			return "", fmt.Errorf("%w: %s must be absolute, got %q", ErrRuntimeDir, envRuntimeDir, v)
		}
		return filepath.Clean(v), nil
	}

	switch runtime.GOOS {
	case "windows":
		base := os.Getenv("LOCALAPPDATA")
		if base == "" {
			return "", fmt.Errorf("%w: LOCALAPPDATA is not set", ErrRuntimeDir)
		}
		return filepath.Join(base, appDirName, "run"), nil

	case "darwin":
		return tempFallback(), nil

	default: // linux and other unix
		if xdg := os.Getenv("XDG_RUNTIME_DIR"); xdg != "" && filepath.IsAbs(xdg) {
			return filepath.Join(xdg, appDirName), nil
		}
		run := filepath.Join("/run/user", strconv.Itoa(os.Getuid()))
		if st, err := os.Stat(run); err == nil && st.IsDir() {
			return filepath.Join(run, appDirName), nil
		}
		return tempFallback(), nil
	}
}

// tempFallback: short name (sockaddr_un limit), per-uid to avoid collisions.
func tempFallback() string {
	return filepath.Join(os.TempDir(), fmt.Sprintf("%s-%d", appDirName, os.Getuid()))
}
