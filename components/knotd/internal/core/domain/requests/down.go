package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type DownRequest struct {
	WorkspaceID values.WorkspaceId
	Services    []values.ServiceName
}
