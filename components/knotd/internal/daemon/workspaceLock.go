package daemon

import (
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type WorkspaceLock struct {
	mu    sync.RWMutex
	locks map[values.WorkspaceId]*sync.Mutex
}

func NewWorkspaceLock(workspaceIds ...values.WorkspaceId) *WorkspaceLock {
	locks := make(map[values.WorkspaceId]*sync.Mutex, len(workspaceIds))
	for _, id := range workspaceIds {
		locks[id] = &sync.Mutex{}
	}

	return &WorkspaceLock{
		locks: locks,
	}
}

func (l *WorkspaceLock) getMutex(id values.WorkspaceId) *sync.Mutex {
	l.mu.RLock()
	m, exists := l.locks[id]
	l.mu.RUnlock()
	if exists {
		return m
	}

	l.mu.Lock()
	defer l.mu.Unlock()

	if m, exists = l.locks[id]; exists {
		return m
	}

	m = &sync.Mutex{}
	l.locks[id] = m
	return m
}

func (l *WorkspaceLock) Lock(id values.WorkspaceId) {
	l.getMutex(id).Lock()
}

func (l *WorkspaceLock) Unlock(id values.WorkspaceId) {
	l.getMutex(id).Unlock()
}
