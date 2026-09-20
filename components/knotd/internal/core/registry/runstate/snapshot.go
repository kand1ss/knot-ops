package runstate

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type RuntimeSnapshot struct {
	Workspace values.WorkspaceId
	Hash      hashing.Hash
	Handles   []ServiceHandle
}
