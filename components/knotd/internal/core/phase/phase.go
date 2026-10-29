package phase

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type PhaseKind uint8

const (
	PhaseKindStop = iota
	PhaseKindStart
)

type PhaseUnit struct {
	Id      values.TaskID
	Service values.ServiceName
	Spec    domain.ServiceSpec // zero for stop units (orphans have no spec)
}

type Phase struct {
	Kind  PhaseKind
	Waves [][]PhaseUnit
}

// Stop phase: names only. Start phase: names + resolved specs.
func NewStopPhase(waves [][]values.ServiceName) Phase {
	p := Phase{Kind: PhaseKindStop, Waves: make([][]PhaseUnit, 0, len(waves))}
	for _, names := range waves {
		w := make([]PhaseUnit, 0, len(names))
		for _, n := range names {
			w = append(w, PhaseUnit{Id: values.NewTaskID(), Service: n})
		}
		p.Waves = append(p.Waves, w)
	}
	return p
}

// Start phase: names + resolved specs.
func NewStartPhase(waves [][]domain.ServiceSpec) Phase {
	p := Phase{Kind: PhaseKindStart, Waves: make([][]PhaseUnit, 0, len(waves))}
	for _, specs := range waves {
		w := make([]PhaseUnit, 0, len(specs))
		for _, s := range specs {
			w = append(w, PhaseUnit{Id: values.NewTaskID(), Service: s.Name, Spec: s})
		}
		p.Waves = append(p.Waves, w)
	}
	return p
}
