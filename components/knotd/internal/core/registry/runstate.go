package registry

import (
	"errors"
	"fmt"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type RuntimeSnapshot struct {
	Workspace values.WorkspaceID
	Hash      hashing.Hash
	Handles   []ServiceHandle
}

// ServiceHandle pairs a handle with the identity needed to act on it
// (which workspace, which service name) — Inspect()/Stop() alone don't
// carry that context, and callers iterating List/ListAll need it.
type ServiceHandle struct {
	Workspace values.WorkspaceID
	Service   values.ServiceName
	Hash      hashing.Hash
	Handle    runtime.RunHandle
}

var ErrAlreadyRegistered = fmt.Errorf("runtime registry: handle already registered for this service")
var ErrHandleNotMatch = errors.New("provided handle doesn't match with saved")

// RuntimeRegistry tracks live RunHandle instances per workspace/service.
// It stores handles (plus the hash of the spec they were started from),
// not status snapshots — Alive/ExitCode are always fetched fresh via
// handle.Inspect(ctx) at query time.
type RuntimeRegistry interface {
	Register(handle ServiceHandle) error
	Get(ws values.WorkspaceID, service values.ServiceName) (ServiceHandle, bool)
	Remove(handle ServiceHandle) error
	List(ws values.WorkspaceID) []ServiceHandle
	ListAll() []ServiceHandle
	ListHashes(ws values.WorkspaceID) map[values.ServiceName]hashing.Hash

	// WorkspaceHash and Snapshot exist specifically to make runtime-drift
	// comparisons against workspace.WorkspaceRecord a plain equality check
	// on hashing.Hash — see the two comparison examples below for how
	// config-drift (Handshake) and runtime-drift (reconciliation) each use
	// exactly one side of this registry.
	WorkspaceHash(ws values.WorkspaceID) (hashing.Hash, bool)
	Snapshot(ws values.WorkspaceID) (RuntimeSnapshot, bool)
}
