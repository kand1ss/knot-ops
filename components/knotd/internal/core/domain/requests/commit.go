package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type CommitRequest struct {
	WorkspaceID values.WorkspaceId
	Manifest    domain.WorkspaceManifest
}

type CommitResponse struct {
	ServicesAdded   []string
	ServicesRemoved []string
	ServicesChanged []string
}
