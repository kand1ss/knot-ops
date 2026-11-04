package supervisor

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"

type EventKind int

const (
	EventExited EventKind = iota
)

type Event struct {
	Handle   registry.ServiceHandle
	Status   EventKind
	ExitCode *int32
}
