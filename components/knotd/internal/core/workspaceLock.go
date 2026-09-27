package core

import (
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type WorkspaceLock struct {
	mu    sync.RWMutex
	locks map[values.WorkspaceId]*sync.RWMutex
}

func NewWorkspaceLock(workspaceIds ...values.WorkspaceId) *WorkspaceLock {
	locks := make(map[values.WorkspaceId]*sync.RWMutex, len(workspaceIds))
	for _, id := range workspaceIds {
		locks[id] = &sync.RWMutex{}
	}

	return &WorkspaceLock{
		locks: locks,
	}
}

func (l *WorkspaceLock) getMutex(id values.WorkspaceId) *sync.RWMutex {
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

	m = &sync.RWMutex{}
	l.locks[id] = m
	return m
}

func (l *WorkspaceLock) Lock(id values.WorkspaceId) {
	l.getMutex(id).Lock()
}

func (l *WorkspaceLock) RLock(id values.WorkspaceId) {
	l.getMutex(id).RLock()
}

func (l *WorkspaceLock) Unlock(id values.WorkspaceId) {
	l.getMutex(id).Unlock()
}

func (l *WorkspaceLock) RUnlock(id values.WorkspaceId) {
	l.getMutex(id).RUnlock()
}
