package domain

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type ServiceSpec struct {
	Name      values.ServiceName
	Command   string
	Directory string
	Depends   []values.ServiceName
	Env       map[string]string
}
