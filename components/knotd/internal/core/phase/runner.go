package phase

import (
	"context"
	"fmt"
	"log/slog"
	"sync"
	"sync/atomic"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/task"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type Stats struct{ started, stopped, skipped, failed atomic.Uint32 }

// emitter selects on the CALLER ctx (consumer liveness), not the merged exec ctx.
// Otherwise a CancelTask (exec ctx done, consumer still listening) would randomly
// drop the very Cancelled events the client is waiting for.
type emitter struct {
	out  chan<- task.TaskEvent
	gone <-chan struct{}
}

func (e emitter) emit(ev task.TaskEvent) {
	select {
	case e.out <- ev:
	case <-e.gone: // consumer is gone: drop instead of leaking goroutine + wsLock
	}
}

type Runner struct {
	wsRegistry registry.WorkspaceRegistry
	rtRegistry registry.RuntimeRegistry
	wsLock     *core.WorkspaceLock
	runtime    runtime.Runtime
}

func NewRunner(
	wsRegistry registry.WorkspaceRegistry,
	rtRegistry registry.RuntimeRegistry,
	wsLock *core.WorkspaceLock,
	runtime runtime.Runtime,
) Runner {
	return Runner{
		rtRegistry: rtRegistry,
		wsRegistry: wsRegistry,
		wsLock:     wsLock,
		runtime:    runtime,
	}
}

func (r *Runner) Execute(
	execCtx, callerCtx context.Context,
	cancel context.CancelFunc,
	taskID values.TaskID,
	wsID values.WorkspaceID,
	phases []Phase,
	out chan task.TaskEvent,
	done func(*Stats) task.TaskEvent,
) {
	defer close(out)
	defer r.wsLock.Unlock(wsID)
	defer cancel()

	em := emitter{out: out, gone: callerCtx.Done()}
	var st Stats

	em.emit(r.createExecutionPlan(taskID, phases))

	for _, p := range phases {
		unavailable := make(map[values.ServiceName]struct{})
		kind := p.kind

		for _, w := range p.waves {
			var wg sync.WaitGroup
			failedThisWave := make(chan values.ServiceName, len(w))

			for _, u := range w {
				if kind == phaseKindStart && execCtx.Err() == nil {
					if dependency, blocked := firstUnavailableDependency(u, unavailable); blocked {
						unavailable[u.service] = struct{}{}
						st.skipped.Add(1)
						em.emit(svcEvent(u, task.SkippedEvent{
							Message: fmt.Sprintf("dependency %q did not start", dependency),
						}))
						continue
					}
				}

				wg.Go(func() {
					if execCtx.Err() != nil {
						em.emit(task.TaskCancelled{TaskID: u.id, Message: "cancelled"})
						return
					}

					switch p.kind {
					case phaseKindStop:
						r.stopUnit(execCtx, wsID, u, em, &st)
					case phaseKindStart:
						if !r.startUnit(execCtx, wsID, u, em, &st) {
							failedThisWave <- u.service
						}
					}
				})
			}
			wg.Wait()
			close(failedThisWave)
			if kind == phaseKindStart {
				for service := range failedThisWave {
					unavailable[service] = struct{}{}
				}
			}
		}
	}

	em.emit(task.TaskDone{
		TaskID:  taskID,
		Started: st.started.Load(),
		Stopped: st.stopped.Load(),
		Skipped: st.skipped.Load(),
		Failed:  st.failed.Load(),
	})
}

func (r *Runner) createExecutionPlan(taskID values.TaskID, phases []Phase) task.TaskAccepted {
	names := make([]values.ServiceName, 0, len(phases))
	for _, phase := range phases {
		for _, wave := range phase.waves {
			for _, unit := range wave {
				names = append(names, unit.service)
			}
		}
	}
	return task.TaskAccepted{
		TaskID:   taskID,
		Services: names,
	}
}

func svcEvent(u PhaseUnit, ev task.Event) task.ServiceEvent {
	return task.ServiceEvent{Service: string(u.service), TaskID: u.id, Event: ev}
}

func failedEvent(u PhaseUnit, err error, ctxMsg, solution string) task.ServiceEvent {
	return svcEvent(u, task.FailedEvent{Error: task.ServiceError{
		Issue: err.Error(), Context: ctxMsg, Solution: solution,
	}})
}

func (r *Runner) startUnit(ctx context.Context, wsID values.WorkspaceID, u PhaseUnit, em emitter, st *Stats) bool {
	if _, exists := r.rtRegistry.Get(wsID, u.service); exists {
		st.skipped.Add(1)
		em.emit(svcEvent(u, task.SkippedEvent{Message: "already running"}))
		return true
	}
	em.emit(svcEvent(u, task.StartingEvent{Message: "starting"}))

	hash, err := hashing.ServiceHash(u.spec)
	if err != nil {
		st.failed.Add(1)
		em.emit(failedEvent(u, err,
			fmt.Sprintf("an error occurred while hashing service %q", u.service),
			fmt.Sprintf("check the %q service configuration", u.service)))
		return false
	}

	handle, err := r.runtime.Start(ctx, u.spec)
	if err != nil {
		st.failed.Add(1)
		em.emit(failedEvent(u, err,
			fmt.Sprintf("an error occurred while starting service %q", u.service),
			fmt.Sprintf("check the %q service configuration, or run 'knot logs'", u.service)))
		return false
	}

	// AlreadyRegistered is NOT benign here: we just spawned a second, untracked process.
	if err := r.rtRegistry.Register(registry.ServiceHandle{
		Workspace: wsID, Service: u.service, Hash: hash, Handle: handle,
	}); err != nil {
		st.failed.Add(1)
		r.stopOrphaned(ctx, handle, stopGrace, wsID, u.service)
		em.emit(failedEvent(u, err,
			fmt.Sprintf("an error occurred while registering service %q", u.service),
			"check for duplicate entries in the configuration, or run 'knot logs'"))
		return false
	}

	st.started.Add(1)
	em.emit(svcEvent(u, task.RunningEvent{Message: fmt.Sprintf("pid: %s", handle.ID())}))
	return true
}

// firstUnavailableDependency returns a direct dependency that failed or was
// skipped earlier in this start plan. Since waves are topologically ordered,
// checking direct dependencies also propagates the block transitively.
func firstUnavailableDependency(
	u PhaseUnit,
	unavailable map[values.ServiceName]struct{},
) (values.ServiceName, bool) {
	for _, dependency := range u.spec.Depends {
		if dependency == "" {
			continue
		}
		if _, exists := unavailable[dependency]; exists {
			return dependency, true
		}
	}
	return "", false
}

// stopOrphaned is a best-effort cleanup for a process that was started but
// could not be safely registered/tracked. Leaving it running untracked is a
// silent resource leak and, worse, an unkillable zombie from knot's own
// perspective (not in rtRegistry -> `knot down` will never see it).
func (r *Runner) stopOrphaned(
	ctx context.Context,
	handle runtime.RunHandle,
	grace time.Duration,
	wsId values.WorkspaceID,
	service values.ServiceName,
) {
	if err := handle.Stop(ctx, grace); err != nil {
		slog.Error("failed to stop orphaned service after registration failure",
			"workspace", wsId, "service", service, "error", err)
	}
}

func (r *Runner) stopUnit(
	ctx context.Context,
	wsID values.WorkspaceID,
	u PhaseUnit,
	em emitter,
	st *Stats,
) {
	em.emit(svcEvent(u, task.StoppingEvent{Message: "stopping"}))

	h, ok := r.rtRegistry.Get(wsID, u.service)
	if !ok { // plan was built from a snapshot; already gone != failure
		st.skipped.Add(1)
		em.emit(svcEvent(u, task.SkippedEvent{Message: "not running"}))
		return
	}

	if err := h.Handle.Stop(ctx, stopGrace); err != nil {
		st.failed.Add(1)
		slog.Error("stop failed", "workspace", wsID, "service", u.service, "error", err)
		em.emit(failedEvent(u, err,
			fmt.Sprintf("an error occurred while stopping service %q", u.service),
			"run 'knot logs'; the process may still be running"))
		return
	}

	// Process is dead. A Remove failure must NOT re-stop it (old code did): just report.
	if err := r.rtRegistry.Remove(wsID, u.service); err != nil {
		st.failed.Add(1)
		em.emit(failedEvent(u, err,
			fmt.Sprintf("service %q stopped, but removing it from the runtime registry failed", u.service),
			"internal daemon error; run 'knot logs'"))
		return
	}

	st.stopped.Add(1)
	em.emit(svcEvent(u, task.StoppedEvent{Message: fmt.Sprintf("pid: %s", h.Handle.ID())}))
}
