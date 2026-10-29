package phase

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/graph"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// BuildDownPhase: requested services + everything that transitively depends on them.
func BuildDownPhase(
	taskID values.TaskID,
	wsID values.WorkspaceId,
	ws workspace.Record,
	requested []values.ServiceName,
) ([]Phase, error) {
	if len(requested) == 0 {
		requested = ws.Manifest.ServiceNames()
	}

	for _, name := range requested { // validation only; the graph is built from the full manifest
		if _, err := ws.Manifest.Get(name); err != nil {
			return nil, err
		}
	}

	// Full manifest: dependents outside `requested` must be visible to the closure.
	dag := graph.NewDependencyGraphFromServices(ws.Manifest.Services())
	nameWaves, err := dag.BuildRevertedWavesFor(requested...)
	if err != nil {
		return nil, err
	}

	return []Phase{NewStopPhase(taskID, nameWaves)}, nil
}
