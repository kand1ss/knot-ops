package runstate

import (
	"fmt"

	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

// ServiceHandle pairs a handle with the identity needed to act on it
// (which workspace, which service name) — Inspect()/Stop() alone don't
// carry that context, and callers iterating List/ListAll need it.
type ServiceHandle struct {
	Workspace values.WorkspaceId
	Service   values.ServiceName
	Hash      hashing.Hash
	Handle    runtime.RunHandle
}

var ErrAlreadyRegistered = fmt.Errorf("runtime registry: handle already registered for this service")
