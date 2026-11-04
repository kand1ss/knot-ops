package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type CommitRequest struct {
	WorkspaceID      values.WorkspaceID
	Manifest         domain.WorkspaceManifest
	ExpectedRevision string
}

type CommitResponse struct {
	Revision        string
	ServicesAdded   []values.ServiceName
	ServicesRemoved []values.ServiceName
	ServicesChanged []values.ServiceName
}
