package workspace

import (
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type InMemoryWorkspaceRegistry struct {
	mu         sync.RWMutex
	workspaces map[values.WorkspaceID]Record
}

func NewInMemoryWorkspaceRegistry() *InMemoryWorkspaceRegistry {
	return &InMemoryWorkspaceRegistry{
		workspaces: make(map[values.WorkspaceID]Record),
	}
}

func (i *InMemoryWorkspaceRegistry) Commit(id values.WorkspaceID, record Record) error {
	i.mu.Lock()
	defer i.mu.Unlock()
	i.workspaces[id] = record
	return nil
}

func (i *InMemoryWorkspaceRegistry) Get(id values.WorkspaceID) (Record, bool) {
	i.mu.RLock()
	defer i.mu.RUnlock()
	m, ok := i.workspaces[id]
	return m, ok
}
