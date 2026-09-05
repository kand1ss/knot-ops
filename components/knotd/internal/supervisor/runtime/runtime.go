package runtime

import (
	"context"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

type Status struct {
	Alive    bool
	ExitCode *int32
	Metadata map[string]string
}

type RunHandle interface {
	ID() string
	Stop(ctx context.Context, grace time.Duration) error
	Inspect(ctx context.Context) (Status, error)
}

type Runtime interface {
	Start(ctx context.Context, service domain.ServiceSpec) (RunHandle, error)
}
