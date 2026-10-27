package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type DownDone struct {
	Stopped uint32
	Failed  uint32
}

func (d DownDone) IsTaskEvent() {}

type DownRequest struct {
	WorkspaceID values.WorkspaceId
	Services    []values.ServiceName
}
