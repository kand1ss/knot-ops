package requests

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type UpEvent interface {
	IsUpEvent()
}

type UpDone struct {
	Started uint32
	Skipped uint32
	Failed  uint32
}

func (u *UpDone) IsUpEvent() {}

type UpCancelled struct {
	Reason  string
	Started uint32
	Stopped uint32
}

func (u *UpCancelled) IsUpEvent() {}

type UpRequest struct {
	WorkspaceId values.WorkspaceId
	Services    []values.ServiceName
}

type UpResponse UpEvent
