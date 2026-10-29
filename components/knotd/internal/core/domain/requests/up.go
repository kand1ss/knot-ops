package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type UpRequest struct {
	WorkspaceId      values.WorkspaceId
	Services         []values.ServiceName
	ExpectedRevision string
	Prune            bool
}
