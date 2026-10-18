package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/task"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type UpDone struct {
	Started uint32
	Skipped uint32
	Failed  uint32
}

func (u UpDone) IsTaskEvent() {}

type UpRequest struct {
	WorkspaceId      values.WorkspaceId
	Services         []values.ServiceName
	ExpectedRevision string
	Prune            bool
}

type UpResponse task.TaskEvent
