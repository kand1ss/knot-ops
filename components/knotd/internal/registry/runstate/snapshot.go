package runstate

import "github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
import "github.com/kand1ss/knot-ops/components/knotd/internal/values"

type RuntimeSnapshot struct {
	Workspace values.WorkspaceId
	Hash      hashing.Hash
	Handles   []ServiceHandle
}
