package requests

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type SyncEvent interface {
	IsSyncEvent()
}

type SyncDone struct {
	ServicesStarted uint32
	ServicesStopped uint32
	ServicesFailed  uint32
}

func (s *SyncDone) IsSyncEvent() {}

type SyncCancelled struct {
	Reason          string
	ServicesStarted uint32
	ServicesStopped uint32
}

func (s *SyncCancelled) IsSyncEvent() {}

type SyncRequest struct {
	WorkspaceID values.WorkspaceId
}

type SyncResponse SyncEvent
