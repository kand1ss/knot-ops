package workspace

import (
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type InMemoryWorkspaceRegistry struct {
	mu         sync.RWMutex
	workspaces map[values.WorkspaceId]Record
}

func NewInMemoryWorkspaceRegistry() *InMemoryWorkspaceRegistry {
	return &InMemoryWorkspaceRegistry{
		workspaces: make(map[values.WorkspaceId]Record),
	}
}

func (i *InMemoryWorkspaceRegistry) Commit(id values.WorkspaceId, record Record) {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.workspaces[id] = record
}

func (i *InMemoryWorkspaceRegistry) Get(id values.WorkspaceId) (Record, bool) {
	i.mu.RLock()
	defer i.mu.RUnlock()
	m, ok := i.workspaces[id]
	return m, ok
}
