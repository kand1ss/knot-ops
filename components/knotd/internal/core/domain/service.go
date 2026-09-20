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

type ServiceMetadata struct {
	port         string
	command      string
	tags         []string
	lastExitCode uint32

	extra map[string]string
}

type ServiceSnapshot struct {
	name         string
	status       ServiceStatus
	pid          uint32
	uptimeSecs   uint64
	restartCount uint32

	metadata ServiceMetadata

	configDrifted bool
}
