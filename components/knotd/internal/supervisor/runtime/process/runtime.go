package process

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor/runtime"
)

var ErrEmptyCommand = errors.New("process runtime: service command is empty")

const forceKillTimeout = 5 * time.Second

// instance is both the RunHandle and the sole owner of *exec.Cmd. It reaps
// the child exactly once via a background goroutine started in Start — the
// moment of death is observed directly through Wait(), never inferred by
// re-polling the OS process table (which would reopen the PID-reuse race
// that knot-sys's fingerprint mechanism exists to close on the Rust side).
type instance struct {
	pid       int
	startedAt time.Time
	proc      *os.Process

	mu       sync.Mutex
	exited   bool
	exitCode *int32
	done     chan struct{}
}

func (i *instance) ID() string {
	return strconv.Itoa(i.pid)
}

func (i *instance) markExited(err error) {
	i.mu.Lock()
	defer i.mu.Unlock()

	if i.exited {
		return
	}
	i.exited = true
	i.exitCode = extractExitCode(err)
	close(i.done)
}

func (i *instance) snapshot() (exited bool, code *int32) {
	i.mu.Lock()
	defer i.mu.Unlock()
	return i.exited, i.exitCode
}

// Stop is idempotent and safe to call concurrently with the reaper
// goroutine and with itself. ctx is expected to be the daemon's own
// lifetime context: if the daemon is force-shutting-down, ctx cancellation
// must be able to interrupt an in-progress graceful wait — hence deriving
// the grace timeout from ctx via context.WithTimeout rather than racing
// ctx.Done() against an independent time.After, which was the previous
// (wrong) approach.
func (i *instance) Stop(ctx context.Context, grace time.Duration) error {
	if exited, _ := i.snapshot(); exited {
		return nil
	}

	if grace > 0 {
		if err := terminateGraceful(i.proc); err != nil && !isProcessDone(err) {
			return fmt.Errorf("process runtime: graceful terminate failed for pid %d: %w", i.pid, err)
		}

		gctx, cancel := context.WithTimeout(ctx, grace)
		defer cancel()

		select {
		case <-i.done:
			return nil
		case <-gctx.Done():
			if ctx.Err() != nil && !errors.Is(gctx.Err(), context.DeadlineExceeded) {
				// Parent (daemon) context was cancelled, not just the grace
				// timeout — the caller is shutting down, not asking us to
				// escalate. Propagate rather than force-kill on their behalf.
				return ctx.Err()
			}
			// Grace period elapsed on its own terms — fall through to force kill.
		}
	}

	if err := killForceful(i.proc); err != nil && !isProcessDone(err) {
		return fmt.Errorf("process runtime: force kill failed for pid %d: %w", i.pid, err)
	}

	fctx, cancel := context.WithTimeout(ctx, forceKillTimeout)
	defer cancel()

	select {
	case <-i.done:
		return nil
	case <-fctx.Done():
		if errors.Is(fctx.Err(), context.DeadlineExceeded) {
			return fmt.Errorf("process runtime: pid %d did not exit after SIGKILL", i.pid)
		}
		return ctx.Err()
	}
}

func (i *instance) Inspect(_ context.Context) (runtime.Status, error) {
	exited, code := i.snapshot()

	return runtime.Status{
		Alive:    !exited,
		ExitCode: code,
		Metadata: map[string]string{
			"runtime":    "process",
			"pid":        i.ID(),
			"started_at": i.startedAt.UTC().Format(time.RFC3339Nano),
		},
	}, nil
}

// Runtime is stateless: it holds no per-service state, it only
// knows how to construct instances. All lifecycle state lives in instance.
type Runtime struct{}

func NewProcessRuntime() *Runtime {
	return &Runtime{}
}

func (r *Runtime) Start(ctx context.Context, service domain.ServiceSpec) (runtime.RunHandle, error) {
	if strings.TrimSpace(service.Command) == "" {
		return nil, fmt.Errorf("%w: service %q", ErrEmptyCommand, service.Name)
	}

	// Only pre-start cancellation is honored here — ctx is the daemon's
	// lifetime context, not scoped to this call, so we don't wire it into
	// cmd.Start() itself (no exec.CommandContext), which would otherwise
	// kill the service the instant *any* daemon-wide cancellation fires,
	// indistinguishable from a deliberate Stop().
	if err := ctx.Err(); err != nil {
		return nil, fmt.Errorf("process runtime: start aborted for service %q: %w", service.Name, err)
	}

	cmd := buildCommand(service)

	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("process runtime: failed to start service %q: %w", service.Name, err)
	}

	inst := &instance{
		pid:       cmd.Process.Pid,
		startedAt: time.Now(),
		proc:      cmd.Process,
		done:      make(chan struct{}),
	}

	go func() {
		err := cmd.Wait()
		inst.markExited(err)
	}()

	return inst, nil
}
