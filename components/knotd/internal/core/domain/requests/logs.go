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

type LogsResponse struct {
	Service   string
	Message   string
	Timestamp time.Time
}
