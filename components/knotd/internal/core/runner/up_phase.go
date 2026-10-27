package runner

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
	Stop  []values.ServiceName
	Start []values.ServiceName
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
		tg.Stop = d.Removed
	}

	if len(req.Services) == 0 {
		tg.Start = ws.Manifest.ServiceNames()
		return tg, nil
	}

	for _, n := range req.Services {
		if _, ok := ws.Manifest.Get(n); ok {
			continue
		}
		return Targets{}, ErrServiceNotExists{WorkspaceID: wsID, Service: n}
	}
	tg.Start = req.Services
	return tg, nil
}

// BuildPrunePhase: all orphans in a single wave, stopped concurrently.
// Dependency order is intentionally NOT honoured: orphans have no spec in the
// manifest, so their edges are unknown without persisting them at start time.
func BuildPrunePhase(
	wsID values.WorkspaceId,
	ws workspace.Record,
	orphans []values.ServiceName,
) ([]Phase, error) {
	if len(orphans) == 0 {
		return nil, nil
	}
	// Sort for a deterministic plan (stable task order in CommandPlan).
	sorted := slices.Clone(orphans)
	slices.Sort(sorted)

	p, err := buildStopPhase(wsID, ws, [][]values.ServiceName{sorted})
	if err != nil {
		return nil, err
	}
	return []Phase{p}, nil
}

func BuildUpPhases(wsID values.WorkspaceId, ws workspace.Record, tg Targets) ([]Phase, error) {
	phases, err := BuildPrunePhase(wsID, ws, tg.Stop)
	if err != nil {
		return nil, err
	}
	if len(tg.Start) == 0 {
		return phases, nil
	}

	g := graph.NewDependencyGraphFromServices(ws.Manifest.Services())
	waves, err := g.BuildWavesFor(tg.Start...)
	if err != nil {
		return nil, err
	}
	start, err := NewPhase(PhaseKindStart, waves, wsID, ws)
	if err != nil {
		return nil, err
	}

	return append(phases, start), nil // prune always precedes start
}
