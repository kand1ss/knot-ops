//go:build windows

package process_test

import (
	"context"
	"path/filepath"
	"syscall"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor/runtime/process"
)

func TestProcessRuntime_Stop_DoesNotOrphanRealPayload(t *testing.T) {
	t.Parallel()

	rt := process.NewProcessRuntime()
	ctx := context.Background()

	pidFile := filepath.Join(t.TempDir(), "payload.pid")
	spec := helperSpecWithPID("MODE_SLEEP", pidFile)

	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	// The PID the payload reports about itself, independent of whatever PID
	// the runtime is tracking (cmd.exe's, in the current implementation).
	payloadPID := waitForHelperPID(t, pidFile)

	if err := handle.Stop(ctx, 500*time.Millisecond); err != nil {
		t.Fatalf("Stop() returned unexpected error: %v", err)
	}

	if !waitUntil(2*time.Second, 20*time.Millisecond, func() bool {
		return !processAlive(payloadPID)
	}) {
		t.Fatalf(
			"payload process (pid %d) is still running 2s after Stop() returned successfully — "+
				"the cmd.exe wrapper was killed but its child process tree was not",
			payloadPID,
		)
	}
}

// processAlive reports whether pid refers to a still-running process, using
// the same low-level Windows API os/exec itself relies on for process
// bookkeeping (no extra dependency needed for this check).
func processAlive(pid int) bool {
	const stillActive = 259 // STILL_ACTIVE, per the Windows API GetExitCodeProcess docs

	handle, err := syscall.OpenProcess(syscall.PROCESS_QUERY_INFORMATION, false, uint32(pid))
	if err != nil {
		// Can't open it — most likely it no longer exists.
		return false
	}
	defer syscall.CloseHandle(handle)

	var exitCode uint32
	if err := syscall.GetExitCodeProcess(handle, &exitCode); err != nil {
		return false
	}
	return exitCode == stillActive
}
