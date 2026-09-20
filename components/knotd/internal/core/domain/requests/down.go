package requests

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type DownEvent interface {
	IsDownEvent()
}

type DownDone struct {
	Stopped uint32
	Failed  uint32
}

func (d *DownDone) IsDownEvent() {}

type DownCancelled struct {
	Reason  string
	Stopped uint32
}

func (d *DownCancelled) IsDownEvent() {}

type DownRequest struct {
	WorkspaceID string
	Services    []values.ServiceName
}

type DownResponse DownEvent
