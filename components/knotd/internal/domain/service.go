package domain

import "github.com/kand1ss/knot-ops/components/knotd/internal/values"

type ServiceSpec struct {
	Name      values.ServiceName
	Command   string
	Directory string
	Depends   []values.ServiceName
	Env       map[string]string
}

type ServiceStatus int

const (
	StatusStopped ServiceStatus = iota
	StatusStarting
	StatusRunning
	StatusWaiting
	StatusFailed
)

type ServiceState struct {
	spec   ServiceSpec
	status ServiceStatus
	PID    uint32
}
