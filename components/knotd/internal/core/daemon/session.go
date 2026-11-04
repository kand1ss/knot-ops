package daemon

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/requests"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/task"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/phase"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type ErrWorkspaceNotExists values.WorkspaceID

func (e ErrWorkspaceNotExists) Error() string {
	return fmt.Sprintf("workspace %q not exists", values.WorkspaceID(e))
}

type ErrWrongRevision struct {
	expected string
	provided string
}

func (e ErrWrongRevision) Error() string {
	return fmt.Sprintf("a provided revision %q doesn't match with the expected %q", e.provided, e.expected)
}

var ErrSessionClosed = errors.New("failed executing: session already closed")

type TaskSession struct {
	id values.TaskID

	ctx    context.Context
	cancel context.CancelFunc

	runner     phase.Runner
	wsRegistry registry.WorkspaceRegistry
	rtRegistry registry.RuntimeRegistry
	wsLock     *core.WorkspaceLock

	onDone   func()
	doneOnce sync.Once

	logger *slog.Logger
}

func NewTask(
	ctx context.Context,
	cancel context.CancelFunc,
	id values.TaskID,
	wsRegistry registry.WorkspaceRegistry,
	rtRegistry registry.RuntimeRegistry,
	wsLock *core.WorkspaceLock,
	runner phase.Runner,
) *TaskSession {
	return &TaskSession{
		ctx:        ctx,
		cancel:     cancel,
		id:         id,
		wsRegistry: wsRegistry,
		rtRegistry: rtRegistry,
		wsLock:     wsLock,
		runner:     runner,
		logger:     slog.Default().With("component", "session", "session_id", id.String()),
	}
}

func (s *TaskSession) Cancel() {
	s.doneOnce.Do(func() {
		if s.onDone != nil {
			s.onDone() // remove from registry first
		}
		s.cancel() // then release ctx-bound resources
	})
}

func (s *TaskSession) withWorkspaceLock(
	id values.WorkspaceID,
	ev chan<- task.TaskEvent,
	fn func(ws workspace.Record) (handoff bool, err error),
) error {
	s.logger.Debug("trying to acquire workspace lock", "workspace", id.String())

	if notLocked := s.wsLock.TryLock(id); !notLocked {
		s.logger.Debug("workspace lock is busy; waiting for lock", "workspace", id.String())
		ev <- task.TaskBlocked{
			TaskID: s.id,
			Reason: task.TaskBlockedReasonWorkspaceBusy,
		}
		s.wsLock.Lock(id)
	}
	s.logger.Debug("workspace lock acquired", "workspace", id.String())

	handedOff := false
	defer func() {
		if !handedOff {
			s.logger.Debug("releasing workspace lock", "workspace", id.String())
			s.wsLock.Unlock(id)
			s.logger.Debug("workspace lock released", "workspace", id.String())
		} else {
			s.logger.Debug("unlock handed off; skip releasing", "workspace", id.String())
		}
	}()

	ws, exists := s.wsRegistry.Get(id)
	if !exists {
		return ErrWorkspaceNotExists(id)
	}

	handedOff, err := fn(ws)
	return err
}

// launch owns lock + ctx handoff; plan is built synchronously.
func (s *TaskSession) launch(
	callerCtx context.Context,
	wsID values.WorkspaceID,
	build func(workspace.Record) ([]phase.Phase, error),
) (<-chan task.TaskEvent, error) {
	if s.ctx.Err() != nil {
		return nil, ErrSessionClosed
	}

	execCtx, cancel := mergeContexts(s.ctx, callerCtx)
	handedOff := false
	defer func() {
		if !handedOff {
			cancel()
			s.Cancel()
		}
	}()

	events := make(chan task.TaskEvent, 10)
	err := s.withWorkspaceLock(wsID, events, func(ws workspace.Record) (bool, error) {
		phases, err := build(ws)
		if err != nil {
			return false, err
		}
		handedOff = true
		go s.runner.Execute(execCtx, callerCtx, cancel, s.id, wsID, phases, events, s.Cancel)
		return true, nil
	})
	if err != nil {
		return nil, err
	}
	return events, nil
}

func mergeContexts(parentCtx, secondaryCtx context.Context) (context.Context, context.CancelFunc) {
	ctx, cancel := context.WithCancel(parentCtx)

	go func() {
		select {
		case <-secondaryCtx.Done():
			cancel()
		case <-ctx.Done():
		}
	}()

	return ctx, cancel
}

// Up validates the request, builds the launch plan, and hands off execution
// to a background goroutine. The returned channel is receive-only from the
// caller's perspective and is guaranteed to be closed once the run completes
// (successfully, on error, or on ctx cancellation).
func (s *TaskSession) Up(ctx context.Context, req requests.UpRequest) (<-chan task.TaskEvent, error) {
	s.logger.InfoContext(ctx, "handle up request",
		"workspace", req.WorkspaceId.String(),
		"services", req.Services,
		"prune", req.Prune,
	)
	return s.launch(ctx, req.WorkspaceId,
		func(ws workspace.Record) ([]phase.Phase, error) {
			if req.ExpectedRevision != ws.Hash.String() {
				return nil, ErrWrongRevision{expected: ws.Hash.String(), provided: req.ExpectedRevision}
			}
			tg, err := phase.ResolveUpTargets(req.WorkspaceId, ws, s.rtRegistry.ListHashes(req.WorkspaceId), req)
			if err != nil {
				return nil, err
			}
			return phase.BuildUpPhases(req.WorkspaceId, ws, tg)
		})
}

func (s *TaskSession) Down(ctx context.Context, req requests.DownRequest) (<-chan task.TaskEvent, error) {
	s.logger.InfoContext(ctx, "handle down request",
		"workspace", req.WorkspaceID.String(),
		"services", req.Services,
	)
	return s.launch(ctx, req.WorkspaceID,
		func(ws workspace.Record) ([]phase.Phase, error) {
			return phase.BuildDownPhase(req.WorkspaceID, ws, req.Services)
		})
}
