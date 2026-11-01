package daemon

import (
	"context"
	"fmt"

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

type TaskSession struct {
	id values.TaskID

	ctx    context.Context
	cancel context.CancelFunc

	runner     phase.Runner
	wsRegistry registry.WorkspaceRegistry
	rtRegistry registry.RuntimeRegistry
	wsLock     *core.WorkspaceLock
}

func NewSession(
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
	}
}

func (s *TaskSession) Cancel() {
	s.cancel()
}

func (s *TaskSession) withWorkspaceLock(
	id values.WorkspaceID,
	fn func(ws workspace.Record) (handoff bool, err error),
) error {
	ws, exists := s.wsRegistry.Get(id)
	if !exists {
		return ErrWorkspaceNotExists(id)
	}

	s.wsLock.Lock(id)
	handedOff := false
	defer func() {
		if !handedOff {
			s.wsLock.Unlock(id)
		}
	}()

	handedOff, err := fn(ws)
	return err
}

// launch owns lock + ctx handoff; plan is built synchronously.
func (s *TaskSession) launch(
	callerCtx context.Context,
	wsID values.WorkspaceID,
	build func(workspace.Record) ([]phase.Phase, error),
) (<-chan task.TaskEvent, error) {
	execCtx, cancel := mergeContexts(s.ctx, callerCtx)
	handedOff := false
	defer func() {
		if !handedOff {
			cancel()
		}
	}()

	var events chan task.TaskEvent
	err := s.withWorkspaceLock(wsID, func(ws workspace.Record) (bool, error) {
		phases, err := build(ws)
		if err != nil {
			return false, err
		}
		events = make(chan task.TaskEvent, 10)
		handedOff = true
		go s.runner.Execute(execCtx, callerCtx, cancel, s.id, wsID, phases, events)
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
	return s.launch(ctx, req.WorkspaceID,
		func(ws workspace.Record) ([]phase.Phase, error) {
			return phase.BuildDownPhase(req.WorkspaceID, ws, req.Services)
		})
}
