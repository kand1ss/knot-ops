package core

import (
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type WorkspaceLock struct {
	mu    sync.Mutex
	locks map[values.WorkspaceId]*sync.RWMutex
}

func NewWorkspaceLock() *WorkspaceLock {
	return &WorkspaceLock{
		locks: make(map[values.WorkspaceId]*sync.RWMutex),
	}
}

func (l *WorkspaceLock) getOrCreate(id values.WorkspaceId) *sync.RWMutex {
	l.mu.Lock()
	defer l.mu.Unlock()

	mtx, exists := l.locks[id]
	if !exists {
		mtx = &sync.RWMutex{}
		l.locks[id] = mtx
	}
	return mtx
}

func (l *WorkspaceLock) Lock(id values.WorkspaceId) {
	l.getOrCreate(id).Lock()
}

func (l *WorkspaceLock) Unlock(id values.WorkspaceId) {
	l.getOrCreate(id).Unlock()
}

func (l *WorkspaceLock) RLock(id values.WorkspaceId) {
	l.getOrCreate(id).RLock()
}

func (l *WorkspaceLock) RUnlock(id values.WorkspaceId) {
	l.getOrCreate(id).RUnlock()
}
