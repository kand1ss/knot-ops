package runner

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/graph"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

func buildStopPhase(
	wsID values.WorkspaceId,
	ws workspace.Record,
	waves [][]values.ServiceName,
) (Phase, error) {
	return NewPhase(PhaseKindStop, waves, wsID, ws)
}

// BuildDownPhase: requested services + everything that transitively depends on them.
func BuildDownPhase(
	wsID values.WorkspaceId,
	ws workspace.Record,
	requested []values.ServiceName,
) ([]Phase, error) {
	if len(requested) == 0 {
		requested = ws.Manifest.ServiceNames()
	}
	for _, name := range requested { // validation only; the graph is built from the full manifest
		if _, ok := ws.Manifest.Get(name); !ok {
			return nil, ErrServiceNotExists{WorkspaceID: wsID, Service: name}
		}
	}

	// Full manifest: dependents outside `requested` must be visible to the closure.
	dag := graph.NewDependencyGraphFromServices(ws.Manifest.Services())
	waves, err := dag.BuildRevertedWavesFor(requested...)
	if err != nil {
		return nil, err
	}

	p, err := buildStopPhase(wsID, ws, waves)
	if err != nil {
		return nil, err
	}
	return []Phase{p}, nil
}
