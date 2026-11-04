package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type UpRequest struct {
	WorkspaceId      values.WorkspaceID
	Services         []values.ServiceName
	ExpectedRevision string
	Prune            bool
}
