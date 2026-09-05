//go:build integration

package supervisor_test

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"sync"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/registry/runstate"
	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor"
	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type realOSProcessHandle struct {
	mu       sync.Mutex
	cmd      *exec.Cmd
	id       string
	exited   bool
	exitCode *int32
}

func startRealProcess(command string, args ...string) (*realOSProcessHandle, error) {
	cmd := exec.Command(command, args...)
	if err := cmd.Start(); err != nil {
		return nil, err
	}

	h := &realOSProcessHandle{
		cmd: cmd,
		id:  cmd.String(),
	}

	go func() {
		err := cmd.Wait()
		h.mu.Lock()
		defer h.mu.Unlock()

		h.exited = true
		if err != nil {
			var exitErr *exec.ExitError
			if errors.As(err, &exitErr) {
				code := int32(exitErr.ExitCode())
				h.exitCode = &code
				return
			}
		}
		code := int32(0)
		h.exitCode = &code
	}()

	return h, nil
}

func (h *realOSProcessHandle) ID() string { return h.id }

func (h *realOSProcessHandle) Stop(_ context.Context, _ time.Duration) error {
	if h.cmd.Process != nil {
		return h.cmd.Process.Kill()
	}
	return nil
}

func (h *realOSProcessHandle) Inspect(_ context.Context) (runtime.Status, error) {
	h.mu.Lock()
	defer h.mu.Unlock()

	return runtime.Status{
		Alive:    !h.exited,
		ExitCode: h.exitCode,
	}, nil
}

func TestIntegration_Supervisor_DetectsRealProcessExit(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	handle, err := startRealProcess("sh", "-c", "exit 42")
	if err != nil {
		t.Skipf("skipping test: system cannot execute shell command: %v", err)
	}

	sh := registry.ServiceHandle{
		Workspace: wsID,
		Service:   "short-lived-worker",
		Handle:    handle,
	}
	_ = reg.Register(sh)

	time.Sleep(50 * time.Millisecond)

	sup := supervisor.NewSupervisor(reg, 20*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	select {
	case ev, ok := <-sup.Events():
		if !ok {
			t.Fatal("events channel closed unexpectedly")
		}
		if ev.Workspace.Service != "short-lived-worker" {
			t.Errorf("expected service 'short-lived-worker', got %s", ev.Workspace.Service)
		}
		if ev.Status != supervisor.EventExited {
			t.Errorf("expected status EventExited, got %v", ev.Status)
		}
		if ev.ExitCode == nil || *ev.ExitCode != 42 {
			t.Errorf("expected exit code 42, got %v", ev.ExitCode)
		}
	case <-time.After(1 * time.Second):
		t.Fatal("timeout waiting for Supervisor to detect exited process")
	}

	_, exists := reg.Get(wsID, "short-lived-worker")
	if exists {
		t.Fatal("exited process was not removed from real registry")
	}
}

func TestIntegration_Supervisor_KeepsRunningProcessUntilKilled(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	handle, err := startRealProcess("sleep", "10")
	if err != nil {
		t.Skipf("skipping test: sleep command failed: %v", err)
	}
	defer func() { _ = handle.Stop(context.Background(), 0) }()

	sh := registry.ServiceHandle{
		Workspace: wsID,
		Service:   "long-running-app",
		Handle:    handle,
	}
	_ = reg.Register(sh)

	sup := supervisor.NewSupervisor(reg, 20*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	select {
	case ev := <-sup.Events():
		t.Fatalf("unexpected event for alive process: %+v", ev)
	case <-time.After(100 * time.Millisecond):
		if _, exists := reg.Get(wsID, "long-running-app"); !exists {
			t.Fatal("alive process was prematurely removed from registry")
		}
	}

	if err := handle.Stop(context.Background(), 0); err != nil {
		t.Fatalf("failed to kill process: %v", err)
	}

	select {
	case ev := <-sup.Events():
		if ev.Workspace.Service != "long-running-app" {
			t.Errorf("expected service 'long-running-app', got %s", ev.Workspace.Service)
		}
	case <-time.After(1 * time.Second):
		t.Fatal("timeout waiting for Supervisor to handle killed process")
	}

	if _, exists := reg.Get(wsID, "long-running-app"); exists {
		t.Fatal("killed process was not removed from registry")
	}
}

func TestIntegration_Supervisor_MultipleParallelProcesses(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	h1, _ := startRealProcess("sh", "-c", "exit 1")
	h2, _ := startRealProcess("sleep", "10")
	defer func() { _ = h2.Stop(context.Background(), 0) }()
	h3, _ := startRealProcess("sh", "-c", "exit 0")

	_ = reg.Register(registry.ServiceHandle{Workspace: wsID, Service: "p1-fail", Handle: h1})
	_ = reg.Register(registry.ServiceHandle{Workspace: wsID, Service: "p2-alive", Handle: h2})
	_ = reg.Register(registry.ServiceHandle{Workspace: wsID, Service: "p3-success", Handle: h3})

	time.Sleep(50 * time.Millisecond)

	sup := supervisor.NewSupervisor(reg, 15*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	exitedServices := make(map[values.ServiceName]int32)

	for i := range 2 {
		select {
		case ev := <-sup.Events():
			if ev.ExitCode != nil {
				exitedServices[ev.Workspace.Service] = *ev.ExitCode
			}
		case <-time.After(1 * time.Second):
			t.Fatalf("timed out waiting for process exit events (received %d/2)", i)
		}
	}

	if code, ok := exitedServices["p1-fail"]; !ok || code != 1 {
		t.Errorf("expected p1-fail to exit with 1, got %v", code)
	}
	if code, ok := exitedServices["p3-success"]; !ok || code != 0 {
		t.Errorf("expected p3-success to exit with 0, got %v", code)
	}

	remaining := reg.ListAll()
	if len(remaining) != 1 || remaining[0].Service != "p2-alive" {
		t.Fatalf("expected only 'p2-alive' to stay in registry, got: %+v", remaining)
	}
}

func TestIntegration_Supervisor_DynamicProcessAddition_DetectsExit(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	sup := supervisor.NewSupervisor(reg, 15*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	time.Sleep(30 * time.Millisecond)

	proc, err := startRealProcess("sh", "-c", "exit 77")
	if err != nil {
		t.Skipf("skipping test: system unable to exec process: %v", err)
	}

	sh := runstate.ServiceHandle{
		Workspace: wsID,
		Service:   "dynamic-late-worker",
		Handle:    proc,
	}

	if err := reg.Register(sh); err != nil {
		t.Fatalf("failed to register handle: %v", err)
	}

	select {
	case ev, ok := <-sup.Events():
		if !ok {
			t.Fatal("events channel closed unexpectedly")
		}
		if ev.Workspace.Service != "dynamic-late-worker" {
			t.Errorf("expected service 'dynamic-late-worker', got %s", ev.Workspace.Service)
		}
		if ev.ExitCode == nil || *ev.ExitCode != 77 {
			t.Errorf("expected exit code 77, got %v", ev.ExitCode)
		}
	case <-time.After(1 * time.Second):
		t.Fatal("timeout waiting for Supervisor to detect dynamically registered process exit")
	}

	if _, exists := reg.Get(wsID, "dynamic-late-worker"); exists {
		t.Fatal("dynamically added process was not removed from registry after exit")
	}
}

func TestIntegration_Supervisor_DynamicProcessAddition_MonitorsAndReactsToTermination(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	sup := supervisor.NewSupervisor(reg, 15*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	proc, err := startRealProcess("sleep", "10")
	if err != nil {
		t.Skipf("skipping test: sleep command failed: %v", err)
	}
	defer func() { _ = proc.Stop(context.Background(), 0) }()

	_ = reg.Register(runstate.ServiceHandle{
		Workspace: wsID,
		Service:   "dynamic-long-running",
		Handle:    proc,
	})

	select {
	case ev := <-sup.Events():
		t.Fatalf("unexpected exit event for running dynamic process: %+v", ev)
	case <-time.After(60 * time.Millisecond):
		if _, exists := reg.Get(wsID, "dynamic-long-running"); !exists {
			t.Fatal("dynamically added running process was prematurely removed from registry")
		}
	}

	if err := proc.Stop(context.Background(), 0); err != nil {
		t.Fatalf("failed to kill process: %v", err)
	}

	select {
	case ev := <-sup.Events():
		if ev.Workspace.Service != "dynamic-long-running" {
			t.Errorf("expected event for 'dynamic-long-running', got %s", ev.Workspace.Service)
		}
	case <-time.After(1 * time.Second):
		t.Fatal("timeout waiting for Supervisor to process dynamic process termination")
	}
}

func TestIntegration_Supervisor_ConcurrentDynamicRegistrations(t *testing.T) {
	t.Parallel()

	wsID := values.NewWorkspaceId()
	reg := runstate.NewInMemoryRuntimeRegistry()

	sup := supervisor.NewSupervisor(reg, 10*time.Millisecond)
	ctx := t.Context()

	go sup.Run(ctx)

	const workerCount = 10
	var wg sync.WaitGroup
	wg.Add(workerCount)

	for i := 0; i < workerCount; i++ {
		go func(id int) {
			defer wg.Done()

			p, err := startRealProcess("sh", "-c", "exit 0")
			if err != nil {
				return
			}

			svcName := values.ServiceName(fmt.Sprintf("concurrent-svc-%d", id))
			_ = reg.Register(runstate.ServiceHandle{
				Workspace: wsID,
				Service:   svcName,
				Handle:    p,
			})
		}(i)
	}

	wg.Wait()

	receivedEvents := make(map[values.ServiceName]bool)
	for i := 0; i < workerCount; i++ {
		select {
		case ev := <-sup.Events():
			receivedEvents[ev.Workspace.Service] = true
		case <-time.After(1 * time.Second):
			t.Fatalf("timeout: received only %d/%d events", len(receivedEvents), workerCount)
		}
	}

	if len(receivedEvents) != workerCount {
		t.Errorf("expected %d unique events, got %d", workerCount, len(receivedEvents))
	}

	if remaining := reg.ListAll(); len(remaining) != 0 {
		t.Fatalf("expected 0 remaining handles in registry, got %d", len(remaining))
	}
}
