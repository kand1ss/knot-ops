package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type StatusRequest struct {
	WorkspaceId values.WorkspaceId
	Services    []values.ServiceName
}

type StatusResponse struct {
	Services []domain.ServiceSnapshot
}
