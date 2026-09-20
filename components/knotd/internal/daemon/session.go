package daemon

import (
	"context"

	"github.com/kand1ss/knot-ops/components/knotd/internal/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type Session struct {
	wsId  values.WorkspaceId
	cmdId string

	ctx    context.Context
	cancel context.CancelFunc

	wsLock     *WorkspaceLock
	wsRegistry *registry.WorkspaceRegistry
	rtRegistry registry.RuntimeRegistry
}

func NewSession(
	wsId values.WorkspaceId,
	cmdId string,
	wsLock *WorkspaceLock,
	wsRegistry *registry.WorkspaceRegistry,
	rtRegistry registry.RuntimeRegistry,
) *Session {
	return &Session{
		wsId:       wsId,
		cmdId:      cmdId,
		wsLock:     wsLock,
		wsRegistry: wsRegistry,
		rtRegistry: rtRegistry,
	}
}

func (s *Session) execute(action func()) {

}
