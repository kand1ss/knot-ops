package runner

import (
	"fmt"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type ErrServiceNotExists struct {
	WorkspaceID values.WorkspaceId
	Service     values.ServiceName
}

func (e ErrServiceNotExists) Error() string {
	return fmt.Sprintf("service %q in workspace %q not exists", e.Service, e.WorkspaceID)
}

type PhaseKind uint8

const (
	PhaseKindStop = iota
	PhaseKindStart
)

type PhaseUnit struct {
	Id      values.TaskID
	Service values.ServiceName
	Spec    domain.ServiceSpec
}

type Phase struct {
	Kind  PhaseKind
	Waves [][]PhaseUnit
}

func NewPhase(kind PhaseKind, waves [][]values.ServiceName,
	wsID values.WorkspaceId, ws workspace.Record) (Phase, error) {

	p := Phase{Kind: kind, Waves: make([][]PhaseUnit, 0, len(waves))}
	for _, names := range waves {
		w := make([]PhaseUnit, 0, len(names))
		for _, n := range names {
			u := PhaseUnit{Id: values.NewTaskID(), Service: n}
			if kind == PhaseKindStart {
				spec, ok := ws.Manifest.Get(n)
				if !ok { // graph/manifest divergence = invariant violation
					return Phase{}, ErrServiceNotExists{WorkspaceID: wsID, Service: n}
				}
				u.Spec = spec
			}
			w = append(w, u)
		}
		p.Waves = append(p.Waves, w)
	}
	return p, nil
}
