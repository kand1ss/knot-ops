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

// TestProcessRuntime_Stop_DoesNotOrphanRealPayload documents a known defect:
// buildCommand on Windows runs the service through "cmd /C <command>", so
// cmd.Process.Pid (what the runtime tracks and what terminateGraceful /
// killForceful call proc.Kill() on) is cmd.exe's PID — not the payload's.
// TerminateProcess against cmd.exe does not tear down its child process
// tree, so the real payload keeps running, untracked, after Stop() reports
// success.
//
// This test is EXPECTED TO FAIL against the current process_windows.go.
// It exists to pin the defect down with a reproducible assertion instead of
// leaving it as a comment, and to turn green once the wrapper-vs-payload
// identity problem is fixed (e.g. via a Job Object that kills the whole
// tree, or by resolving and signaling the actual child PID directly).
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
