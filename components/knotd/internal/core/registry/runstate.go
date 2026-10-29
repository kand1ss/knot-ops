package registry

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/runstate"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// RuntimeRegistry tracks live RunHandle instances per workspace/service.
// It stores handles (plus the hash of the spec they were started from),
// not status snapshots — Alive/ExitCode are always fetched fresh via
// handle.Inspect(ctx) at query time.
type RuntimeRegistry interface {
	Register(handle runstate.ServiceHandle) error
	Get(ws values.WorkspaceID, service values.ServiceName) (runstate.ServiceHandle, bool)
	Remove(ws values.WorkspaceID, service values.ServiceName) error
	List(ws values.WorkspaceID) []runstate.ServiceHandle
	ListAll() []runstate.ServiceHandle
	ListHashes(ws values.WorkspaceID) map[values.ServiceName]hashing.Hash

	// WorkspaceHash and Snapshot exist specifically to make runtime-drift
	// comparisons against workspace.WorkspaceRecord a plain equality check
	// on hashing.Hash — see the two comparison examples below for how
	// config-drift (Handshake) and runtime-drift (reconciliation) each use
	// exactly one side of this registry.
	WorkspaceHash(ws values.WorkspaceID) (hashing.Hash, bool)
	Snapshot(ws values.WorkspaceID) (runstate.RuntimeSnapshot, bool)
}

type ServiceHandle = runstate.ServiceHandle
type RuntimeSnapshot = runstate.RuntimeSnapshot
