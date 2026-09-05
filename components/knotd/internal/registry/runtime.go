package registry

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/registry/runstate"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

// RuntimeRegistry tracks live RunHandle instances per workspace/service.
// It stores handles (plus the hash of the spec they were started from),
// not status snapshots — Alive/ExitCode are always fetched fresh via
// handle.Inspect(ctx) at query time.
type RuntimeRegistry interface {
	Register(handle runstate.ServiceHandle) error
	Get(ws values.WorkspaceId, service values.ServiceName) (runstate.ServiceHandle, bool)
	Remove(ws values.WorkspaceId, service values.ServiceName)
	List(ws values.WorkspaceId) []runstate.ServiceHandle
	ListAll() []runstate.ServiceHandle

	// WorkspaceHash and Snapshot exist specifically to make runtime-drift
	// comparisons against workspace.WorkspaceRecord a plain equality check
	// on hashing.Hash — see the two comparison examples below for how
	// config-drift (Handshake) and runtime-drift (reconciliation) each use
	// exactly one side of this registry.
	WorkspaceHash(ws values.WorkspaceId) (hashing.Hash, bool)
	Snapshot(ws values.WorkspaceId) (runstate.RuntimeSnapshot, bool)
}

type ServiceHandle = runstate.ServiceHandle
type RuntimeSnapshot = runstate.RuntimeSnapshot
