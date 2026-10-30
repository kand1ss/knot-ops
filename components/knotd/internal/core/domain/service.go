package domain

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type ServiceStatus int

const (
	ServiceStatusRunning = iota
	ServiceStatusStopped
	ServiceStatusFailed
	ServiceStatusStarting
	ServiceStatusStopping
)

type ServiceSpec struct {
	Name      values.ServiceName
	Command   string
	Directory string
	Depends   []values.ServiceName

	Env map[string]string
}

type ServiceSnapshot struct {
	Name       values.ServiceName
	Status     ServiceStatus
	Pid        uint32
	UptimeSecs uint64

	Drifted  bool
	Orphaned bool

	Extra map[string]string
}
