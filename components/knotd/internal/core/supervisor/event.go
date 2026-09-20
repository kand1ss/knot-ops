package supervisor

import "github.com/kand1ss/knot-ops/components/knotd/internal/registry/runstate"

type EventKind int

const (
	EventExited EventKind = iota
)

type Event struct {
	Workspace runstate.ServiceHandle
	Status    EventKind
	ExitCode  *int32
}
