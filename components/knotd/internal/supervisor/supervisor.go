package supervisor

import (
	"context"
	"log/slog"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/registry"
)

// Supervisor periodically inspects every handle currently tracked in
// RuntimeRegistry and reacts to processes that have died. It does not
// store handles itself (RuntimeRegistry already is that source of truth)
// and does not start anything (that stays with the Up command handler,
// which is the only place that knows how to route a ServiceSpec to the
// correct Runtime implementation).
type Supervisor struct {
	runtimes        registry.RuntimeRegistry
	inspectInterval time.Duration
	events          chan Event
}

func NewSupervisor(runtimes registry.RuntimeRegistry, inspectInterval time.Duration) *Supervisor {
	return &Supervisor{
		runtimes:        runtimes,
		inspectInterval: inspectInterval,
		events:          make(chan Event, 64),
	}
}

// Events returns the read side of the event stream. Consumers (e.g. the
// daemon's central event bus feeding gRPC streams) subscribe here.
func (s *Supervisor) Events() <-chan Event {
	return s.events
}

// Run drives the poll loop until ctx is cancelled — ctx here is correctly
// the daemon's own lifetime context, not a per-request one, since
// supervision must outlive any single RPC.
func (s *Supervisor) Run(ctx context.Context) {
	ticker := time.NewTicker(s.inspectInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			close(s.events)
			return
		case <-ticker.C:
			s.tick(ctx)
		}
	}
}

func (s *Supervisor) tick(ctx context.Context) {
	for _, sh := range s.runtimes.ListAll() {
		status, err := sh.Handle.Inspect(ctx)
		if err != nil {
			slog.Error("inspect failed", "service", sh.Service, "workspace", sh.Workspace, "err", err)
			continue
		}

		if status.Alive {
			continue
		}

		// Transition detected: process has exited since last tick.
		// RuntimeRegistry must stop claiming this service is running —
		// this is the single point where that claim gets corrected.
		s.runtimes.Remove(sh.Workspace, sh.Service)

		select {
		case s.events <- Event{Workspace: sh, Status: EventExited, ExitCode: status.ExitCode}:
		default:
			slog.Warn("event channel full, dropping exit event", "service", sh.Service)
		}
	}
}
