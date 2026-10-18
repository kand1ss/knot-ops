package requests

import (
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type LogsRequest struct {
	WorkspaceId values.WorkspaceId
	Services    []values.ServiceName
	Follow      bool
	Tail        uint32
}

type LogEntry struct {
	Message   string
	Timestamp time.Time
}

type LogEvent interface {
	IsLogEvent()
}

type LogBatch struct {
	Entries []LogEntry
	Dropped uint32
}

func (l LogBatch) IsLogEvent() {}

type TailComplete struct{}

func (t TailComplete) IsLogEvent() {}

type LogsResponse struct {
	Service values.ServiceName
	Event   LogEvent
}
