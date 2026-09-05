package registry

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type WorkspaceRegistry interface {
	// Commit persists the record produced by a successful Sync. Must NOT
	// be called from Handshake — Handshake only reads (Get) to compute
	// drift; writing here before Sync actually reconciles runtime state
	// would let the daemon believe a manifest is applied when it isn't.
	Commit(id values.WorkspaceId, record workspace.Record)
	Get(id values.WorkspaceId) (workspace.Record, bool)
}
