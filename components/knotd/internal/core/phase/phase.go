package phase

import (
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

const stopGrace = 300 * time.Millisecond

type phaseKind uint8

const (
	phaseKindStop = iota
	phaseKindStart
)

type PhaseUnit struct {
	id      values.TaskID
	service values.ServiceName
	spec    domain.ServiceSpec // zero for stop units (orphans have no spec)
}

type Phase struct {
	kind  phaseKind
	waves [][]PhaseUnit
}

// Stop phase: names only. Start phase: names + resolved specs.
func NewStopPhase(waves [][]values.ServiceName) Phase {
	p := Phase{kind: phaseKindStop, waves: make([][]PhaseUnit, 0, len(waves))}
	for _, names := range waves {
		w := make([]PhaseUnit, 0, len(names))
		for _, n := range names {
			w = append(w, PhaseUnit{id: values.NewTaskID(), service: n})
		}
		p.waves = append(p.waves, w)
	}
	return p
}

// Start phase: names + resolved specs.
func NewStartPhase(waves [][]domain.ServiceSpec) Phase {
	p := Phase{kind: phaseKindStart, waves: make([][]PhaseUnit, 0, len(waves))}
	for _, specs := range waves {
		w := make([]PhaseUnit, 0, len(specs))
		for _, s := range specs {
			w = append(w, PhaseUnit{id: values.NewTaskID(), service: s.Name, spec: s})
		}
		p.waves = append(p.waves, w)
	}
	return p
}
