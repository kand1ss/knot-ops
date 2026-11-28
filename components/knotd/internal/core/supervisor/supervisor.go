package supervisor

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync/atomic"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
)

var ErrNegativeValue = fmt.Errorf("negative value is provided but not supported")

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

	reported map[runtime.RunHandle]struct{}
	dropped  atomic.Uint32

	logger *slog.Logger
}

func NewSupervisor(runtimes registry.RuntimeRegistry, inspectInterval time.Duration) (*Supervisor, error) {
	if inspectInterval <= 0 {
		return nil, ErrNegativeValue
	}

	if runtimes == nil {
		return nil, errors.New("runtime registry must not be nil")
	}

	return &Supervisor{
		runtimes:        runtimes,
		inspectInterval: inspectInterval,
		events:          make(chan Event, 64),
		reported:        make(map[runtime.RunHandle]struct{}),
		logger:          slog.Default().With("component", "supervisor"),
	}, nil
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
	live := make(map[runtime.RunHandle]struct{})
	for _, sh := range s.runtimes.ListAll() {
		live[sh.Handle] = struct{}{}
		if _, done := s.reported[sh.Handle]; done {
			continue
		}

		st, err := sh.Handle.Inspect(ctx)
		if err != nil {
			s.logger.ErrorContext(ctx, "inspect failed", "service", sh.Service, "workspace", sh.Workspace.String(), "error", err)
			continue
		}

		if st.Alive {
			continue
		}

		select {
		case s.events <- Event{Handle: sh, Status: EventExited, ExitCode: st.ExitCode}:
			s.reported[sh.Handle] = struct{}{} // only after delivery
		case <-ctx.Done():
			s.logger.ErrorContext(ctx,
				"failed to deliver event: context was cancelled",
				"service", sh.Service,
				"workspace", sh.Workspace.String(),
				"error", ctx.Err(),
			)
			return
		default:
			s.dropped.Add(1)
			s.logger.WarnContext(ctx, "supervisor channel is full: failed to deliver event; retry next interval",
				"service", sh.Service,
				"workspace", sh.Workspace.String(),
				"dropped_count", s.dropped.Load(),
			)
		}
	}

	for k := range s.reported { // GC
		if _, ok := live[k]; !ok {
			delete(s.reported, k)
		}
	}
}
