package phase

import (
	"slices"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/diff"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/requests"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/graph"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type Targets struct {
	stop  []values.ServiceName
	start []values.ServiceName
}

func ResolveUpTargets(
	wsID values.WorkspaceId,
	ws workspace.Record,
	rtHashes map[values.ServiceName]hashing.Hash,
	req requests.UpRequest,
) (Targets, error) {
	var tg Targets

	if req.Prune {
		d := diff.ComputeDiff(rtHashes, ws.ServiceHashes)
		tg.stop = d.Removed
	}

	if len(req.Services) == 0 {
		tg.start = ws.Manifest.ServiceNames()
		return tg, nil
	}

	for _, n := range req.Services {
		if _, err := ws.Manifest.Get(n); err != nil {
			return Targets{}, err
		}
	}

	tg.start = req.Services
	return tg, nil
}

// BuildPrunePhase: all orphans in a single wave, stopped concurrently.
// Dependency order is intentionally NOT honoured: orphans have no spec in the
// manifest, so their edges are unknown without persisting them at start time.
func BuildPrunePhase(orphans []values.ServiceName) []Phase {
	if len(orphans) == 0 {
		return nil
	}
	// Sort for a deterministic plan (stable task order in CommandPlan).
	sorted := slices.Clone(orphans)
	slices.Sort(sorted)

	return []Phase{NewStopPhase([][]values.ServiceName{sorted})}
}

func BuildUpPhases(wsID values.WorkspaceId, ws workspace.Record, tg Targets) ([]Phase, error) {
	phases := BuildPrunePhase(tg.stop)
	if len(tg.start) == 0 {
		return phases, nil
	}

	g := graph.NewDependencyGraphFromServices(ws.Manifest.Services())
	nameWaves, err := g.BuildWavesFor(tg.start...)
	if err != nil {
		return nil, err
	}

	specWaves, err := ws.Manifest.SpecWaves(nameWaves)
	if err != nil {
		return nil, err
	}

	return append(phases, NewStartPhase(specWaves)), nil // prune always precedes start
}
