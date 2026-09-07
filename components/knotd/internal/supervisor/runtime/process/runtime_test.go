package process_test

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor/runtime/process"
)

func TestHelperProcess(t *testing.T) {
	var mode, readyFile, pidFile string

	for _, arg := range os.Args {
		switch {
		case arg == "MODE_EXIT_42", arg == "MODE_IGNORE_TERM", arg == "MODE_SLEEP":
			mode = arg
		case strings.HasPrefix(arg, "READY_FILE="):
			readyFile = strings.TrimPrefix(arg, "READY_FILE=")
		case strings.HasPrefix(arg, "PID_FILE="):
			pidFile = strings.TrimPrefix(arg, "PID_FILE=")
		}
	}

	// Report our own PID before doing anything else. This is how a test can
	// tell "the wrapper's PID" (cmd.exe / sh, tracked by the runtime) apart
	// from "the PID that actually ran the payload" — the two are only
	// guaranteed identical on unix, and only after the exec-prefix fix.
	if pidFile != "" {
		if err := os.WriteFile(pidFile, []byte(strconv.Itoa(os.Getpid())), 0o644); err != nil {
			panic(fmt.Sprintf("failed to write pid file: %v", err))
		}
	}

	switch mode {
	case "MODE_EXIT_42":
		os.Exit(42)

	case "MODE_IGNORE_TERM":
		sigCh := make(chan os.Signal, 1)
		signal.Notify(sigCh, syscall.SIGTERM)

		// Only announce readiness AFTER Notify has taken effect. Without this,
		// the parent has no way to know when the handler is actually installed,
		// so a SIGTERM sent too early hits the default disposition (terminate)
		// instead of being intercepted — which is exactly why the escalation
		// path was flaky: the "graceful" signal was killing the process outright.
		if readyFile != "" {
			if err := os.WriteFile(readyFile, []byte("ready"), 0o644); err != nil {
				panic(fmt.Sprintf("failed to write ready file: %v", err))
			}
		}
		select {}

	case "MODE_SLEEP":
		time.Sleep(10 * time.Minute)
	}
}

func helperSpec(mode string) domain.ServiceSpec {
	return helperSpecWithArgs(mode)
}

// helperSpecWithReady builds a spec for MODE_IGNORE_TERM that writes readyFile
// once the child has registered its SIGTERM handler. Callers MUST wait on that
// file before sending any signal — see waitForHelperReady.
func helperSpecWithReady(mode, readyFile string) domain.ServiceSpec {
	return helperSpecWithArgs(mode, "READY_FILE="+readyFile)
}

// helperSpecWithPID builds a spec that writes the payload's own PID to
// pidFile as soon as it starts, independent of whatever PID the runtime
// itself is tracking (cmd.exe / sh). Use with waitForHelperPID to get the
// real payload PID for out-of-band liveness checks after Stop().
func helperSpecWithPID(mode, pidFile string) domain.ServiceSpec {
	return helperSpecWithArgs(mode, "PID_FILE="+pidFile)
}

func helperSpecWithArgs(mode string, extraArgs ...string) domain.ServiceSpec {
	execPath, err := os.Executable()
	if err != nil {
		panic(fmt.Sprintf("failed to get os.Executable: %v", err))
	}

	args := append([]string{mode}, extraArgs...)
	return domain.ServiceSpec{
		Name:    "test-service",
		Command: fmt.Sprintf("%q -test.run=^TestHelperProcess$ -- %s", execPath, strings.Join(args, " ")),
	}
}

// waitForHelperReady blocks until the helper process has confirmed (via
// readyFile) that its signal handler is installed, or fails the test.
// This replaces a fixed time.Sleep, which is a race by construction: no
// sleep duration is provably sufficient under CI scheduling jitter, GOMAXPROCS
// contention, or -race instrumentation overhead.
func waitForHelperReady(t *testing.T, readyFile string) {
	t.Helper()
	if !waitUntil(2*time.Second, 5*time.Millisecond, func() bool {
		_, statErr := os.Stat(readyFile)
		return statErr == nil
	}) {
		t.Fatalf("timed out waiting for helper process to install its SIGTERM handler (ready file: %s)", readyFile)
	}
}

// waitForHelperPID blocks until the helper process has reported its own PID
// via pidFile, and returns it. Fails the test on timeout or malformed content.
func waitForHelperPID(t *testing.T, pidFile string) int {
	t.Helper()

	var content []byte
	if !waitUntil(2*time.Second, 5*time.Millisecond, func() bool {
		data, statErr := os.ReadFile(pidFile)
		if statErr != nil || len(data) == 0 {
			return false
		}
		content = data
		return true
	}) {
		t.Fatalf("timed out waiting for helper process to report its pid (pid file: %s)", pidFile)
	}

	pid, err := strconv.Atoi(strings.TrimSpace(string(content)))
	if err != nil {
		t.Fatalf("helper process wrote malformed pid %q: %v", content, err)
	}
	return pid
}

func newSpec(cmd string) domain.ServiceSpec {
	return domain.ServiceSpec{
		Name:    "test-service",
		Command: cmd,
	}
}

func TestProcessRuntime_Start_Validation(t *testing.T) {
	t.Parallel()

	rt := process.NewProcessRuntime()

	t.Run("returns error on empty command", func(t *testing.T) {
		t.Parallel()

		spec := newSpec("   ")
		handle, err := rt.Start(t.Context(), spec)

		if !errors.Is(err, process.ErrEmptyCommand) {
			t.Fatalf("expected ErrEmptyCommand, got: %v", err)
		}
		if handle != nil {
			t.Fatal("expected handle to be nil on error")
		}
	})

	t.Run("returns error if context is already canceled pre-start", func(t *testing.T) {
		t.Parallel()

		ctx, cancel := context.WithCancel(context.Background())
		cancel()

		spec := helperSpec("MODE_SLEEP")
		handle, err := rt.Start(ctx, spec)

		if err == nil {
			t.Fatal("expected error for pre-canceled context, got nil")
		}
		if handle != nil {
			t.Fatal("expected handle to be nil on error")
		}
	})
}

func TestProcessRuntime_Lifecycle_InspectAndExit(t *testing.T) {
	t.Parallel()

	rt := process.NewProcessRuntime()
	ctx := t.Context()

	spec := helperSpec("MODE_EXIT_42")
	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	if handle.ID() == "" || handle.ID() == "0" {
		t.Errorf("expected valid PID in ID(), got: %q", handle.ID())
	}

	success := waitUntil(2*time.Second, 10*time.Millisecond, func() bool {
		st, err := handle.Inspect(ctx)
		if err != nil {
			t.Errorf("Inspect returned error: %v", err)
			return false
		}
		if !st.Alive {
			if st.ExitCode != nil && *st.ExitCode == 42 {
				return true
			}
		}
		return false
	})

	if !success {
		st, _ := handle.Inspect(ctx)
		var code int32 = -1
		if st.ExitCode != nil {
			code = *st.ExitCode
		}
		t.Fatalf("expected process to exit with code 42, got Alive=%v, ExitCode=%d", st.Alive, code)
	}

	st, err := handle.Inspect(ctx)
	if err != nil {
		t.Fatalf("Inspect failed: %v", err)
	}
	if st.Metadata["runtime"] != "process" {
		t.Errorf("expected metadata runtime=process, got %s", st.Metadata["runtime"])
	}
	if st.Metadata["pid"] != handle.ID() {
		t.Errorf("expected metadata pid=%s, got %s", handle.ID(), st.Metadata["pid"])
	}
}

func TestProcessRuntime_Stop_Graceful(t *testing.T) {
	t.Parallel()

	rt := process.NewProcessRuntime()
	ctx := context.Background()

	spec := helperSpec("MODE_SLEEP")
	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	st, err := handle.Inspect(ctx)
	if err != nil || !st.Alive {
		t.Fatalf("expected process to be alive, status: %+v, err: %v", st, err)
	}

	stopErr := handle.Stop(ctx, 1*time.Second)
	if stopErr != nil {
		t.Fatalf("Stop() returned unexpected error: %v", stopErr)
	}

	st, err = handle.Inspect(ctx)
	if err != nil {
		t.Fatalf("Inspect failed: %v", err)
	}
	if st.Alive {
		t.Fatal("expected process to be stopped after Stop()")
	}
}

func TestProcessRuntime_Stop_EscalateToForceKill(t *testing.T) {
	t.Parallel()

	if runtime.GOOS == "windows" {
		t.Skip("Windows does not support trapping/ignoring SIGTERM signals")
	}

	rt := process.NewProcessRuntime()
	ctx := context.Background()

	readyFile := filepath.Join(t.TempDir(), "ready")
	spec := helperSpecWithReady("MODE_IGNORE_TERM", readyFile)
	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	waitForHelperReady(t, readyFile)

	start := time.Now()
	gracePeriod := 100 * time.Millisecond
	err = handle.Stop(ctx, gracePeriod)
	elapsed := time.Since(start)

	if err != nil {
		t.Fatalf("Stop() returned unexpected error: %v", err)
	}

	if elapsed < gracePeriod {
		t.Errorf("expected Stop() to wait at least %v for grace period, took %v", gracePeriod, elapsed)
	}

	st, inspectErr := handle.Inspect(ctx)
	if inspectErr != nil || st.Alive {
		t.Fatalf("expected process to be killed by SIGKILL, status: %+v", st)
	}
}

func TestProcessRuntime_Stop_ParentContextCanceled(t *testing.T) {
	t.Parallel()

	if runtime.GOOS == "windows" {
		t.Skip("Windows does not support graceful SIGTERM grace periods")
	}

	rt := process.NewProcessRuntime()

	readyFile := filepath.Join(t.TempDir(), "ready")
	spec := helperSpecWithReady("MODE_IGNORE_TERM", readyFile)
	ctx, cancel := context.WithCancel(context.Background())

	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	waitForHelperReady(t, readyFile)

	errCh := make(chan error, 1)
	go func() {
		errCh <- handle.Stop(ctx, 5*time.Second)
	}()

	time.Sleep(50 * time.Millisecond)
	cancel()

	select {
	case stopErr := <-errCh:
		if !errors.Is(stopErr, context.Canceled) {
			t.Fatalf("expected context.Canceled error when parent ctx is canceled, got: %v", stopErr)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("timeout waiting for Stop() to return after context cancellation")
	}

	_ = handle.Stop(context.Background(), 0)
}

func TestProcessRuntime_Stop_IdempotencyAndConcurrency(t *testing.T) {
	t.Parallel()

	rt := process.NewProcessRuntime()
	ctx := context.Background()

	spec := helperSpec("MODE_SLEEP")
	handle, err := rt.Start(ctx, spec)
	if err != nil {
		t.Fatalf("failed to start process: %v", err)
	}

	const goroutines = 10
	var wg sync.WaitGroup
	errs := make([]error, goroutines)

	wg.Add(goroutines)
	for i := range goroutines {
		go func(idx int) {
			defer wg.Done()
			errs[idx] = handle.Stop(ctx, 100*time.Millisecond)
		}(i)
	}

	wg.Wait()

	for i, err := range errs {
		if err != nil {
			t.Errorf("goroutine %d returned error on Stop(): %v", i, err)
		}
	}

	if err := handle.Stop(ctx, 0); err != nil {
		t.Errorf("repeated Stop() on stopped process returned error: %v", err)
	}
}

func waitUntil(timeout, pollInterval time.Duration, condition func() bool) bool {
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if condition() {
			return true
		}
		time.Sleep(pollInterval)
	}
	return condition()
}
