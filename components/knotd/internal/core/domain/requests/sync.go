package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
)

type SyncEvent interface {
	IsSyncEvent()
}

type SyncResult struct {
	ServicesAdded   []string
	ServicesChanged []string
	ServicesRemoved []string
}

func (s *SyncResult) IsSyncEvent() {}

type SyncRequest struct {
	Manifest domain.WorkspaceManifest
	Metadata domain.WorkspaceMetadata
}

type SyncResponse SyncEvent
