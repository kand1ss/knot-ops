package runstate

import (
	"fmt"
	"sync"

	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type InMemoryRuntimeRegistry struct {
	mu sync.RWMutex
	// workspace -> service name -> handle
	byWorkspace map[values.WorkspaceId]map[values.ServiceName]ServiceHandle
}

func NewInMemoryRuntimeRegistry() *InMemoryRuntimeRegistry {
	return &InMemoryRuntimeRegistry{
		byWorkspace: make(map[values.WorkspaceId]map[values.ServiceName]ServiceHandle),
	}
}

func (r *InMemoryRuntimeRegistry) Register(handle ServiceHandle) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	services, ok := r.byWorkspace[handle.Workspace]
	if !ok {
		services = make(map[values.ServiceName]ServiceHandle)
		r.byWorkspace[handle.Workspace] = services
	}

	if _, exists := services[handle.Service]; exists {
		return fmt.Errorf("%w: workspace=%s service=%s", ErrAlreadyRegistered, handle.Workspace, handle.Service)
	}

	services[handle.Service] = handle
	return nil
}

// WorkspaceHash computes the combined hash of every service currently
// tracked for ws, via the exact same Combine used for declared manifests.
// Cheap: allocates only the small pairs slice, no handle copies. This is
// the method Handshake-style "is anything even worth inspecting further"
// checks should use.
//
// ok=false means "nothing tracked for this workspace" — not "hash is
// zero" (Combine(nil) is a well-defined, non-zero value in general, so the
// two cases can't be confused by checking the hash alone).
func (r *InMemoryRuntimeRegistry) WorkspaceHash(ws values.WorkspaceId) (hashing.Hash, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	services, ok := r.byWorkspace[ws]
	if !ok || len(services) == 0 {
		return hashing.Hash{}, false
	}

	pairs := make([]hashing.NamedHash, 0, len(services))
	for name, h := range services {
		pairs = append(pairs, hashing.NamedHash{Name: string(name), Hash: h.Hash})
	}
	return hashing.Combine(pairs), true
}

// Snapshot returns both the handle list and the combined hash for ws,
// computed under a single lock acquisition. Prefer this over calling
// WorkspaceHash and List separately when both are needed (e.g. Sync,
// which wants the diff AND the summary hash) — two separate calls could
// observe two different states if a Register/Remove lands between them,
// which would make the returned hash inconsistent with the returned
// handle list. This is the completed, correctly-shaped replacement for
// the abandoned GetWorkspace() stub.
func (r *InMemoryRuntimeRegistry) Snapshot(ws values.WorkspaceId) (RuntimeSnapshot, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	services, ok := r.byWorkspace[ws]
	if !ok || len(services) == 0 {
		return RuntimeSnapshot{}, false
	}

	handles := make([]ServiceHandle, 0, len(services))
	pairs := make([]hashing.NamedHash, 0, len(services))
	for name, h := range services {
		handles = append(handles, h)
		pairs = append(pairs, hashing.NamedHash{Name: string(name), Hash: h.Hash})
	}

	return RuntimeSnapshot{
		Workspace: ws,
		Hash:      hashing.Combine(pairs),
		Handles:   handles,
	}, true
}

func (r *InMemoryRuntimeRegistry) Get(ws values.WorkspaceId, service values.ServiceName) (ServiceHandle, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	services, ok := r.byWorkspace[ws]
	if !ok {
		return ServiceHandle{}, false
	}
	h, ok := services[service]
	return h, ok
}

func (r *InMemoryRuntimeRegistry) Remove(ws values.WorkspaceId, service values.ServiceName) {
	r.mu.Lock()
	defer r.mu.Unlock()

	services, ok := r.byWorkspace[ws]
	if !ok {
		return
	}
	delete(services, service)

	// Prevent unbounded growth of byWorkspace itself: a workspace that's
	// been fully torn down (`knot down`) shouldn't leave an empty map
	// entry behind forever.
	if len(services) == 0 {
		delete(r.byWorkspace, ws)
	}
}

func (r *InMemoryRuntimeRegistry) List(ws values.WorkspaceId) []ServiceHandle {
	r.mu.RLock()
	defer r.mu.RUnlock()

	services, ok := r.byWorkspace[ws]
	if !ok {
		return nil
	}

	out := make([]ServiceHandle, 0, len(services))
	for _, h := range services {
		out = append(out, h)
	}

	return out
}

func (r *InMemoryRuntimeRegistry) ListAll() []ServiceHandle {
	r.mu.RLock()
	defer r.mu.RUnlock()

	total := 0
	for _, services := range r.byWorkspace {
		total += len(services)
	}

	out := make([]ServiceHandle, 0, total)
	for _, services := range r.byWorkspace {
		for _, h := range services {
			out = append(out, h)
		}
	}
	return out
}
