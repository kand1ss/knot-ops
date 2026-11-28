package phase

import (
	"cmp"
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

// ResolveUpTargets computes which services must be stopped and started.
//
// Semantics:
//   - scope  = req.Services, or the whole manifest when empty (validated first).
//   - start  = scope. The executor skips services already in the desired state,
//     so a service stopped during the stop phase is started again.
//   - stop   = (prune ? orphans : ∅) ∪ (restart ? scope ∩ running : ∅).
//     Restart never touches services outside scope, nor orphans.
//
// Output slices are fresh, deduplicated and sorted; actual ordering for
// execution is the DAG engine's responsibility.
func ResolveUpTargets(
	ws workspace.Record,
	rtHashes map[values.ServiceName]hashing.Hash,
	req requests.UpRequest,
) (Targets, error) {
	// 1. Resolve and validate scope first (fail fast, no wasted diff work).
	scope, err := resolveScope(ws, req.Services)
	if err != nil {
		return Targets{}, err
	}

	var tg Targets
	tg.start = scope

	// 2. Prune is intentionally global: orphans cannot be named in req.Services
	//    because they are absent from the manifest.
	if req.Prune {
		tg.stop = append(tg.stop, diff.ComputeDiff(rtHashes, ws.ServiceHashes).Removed...)
	}

	// 3. Restart is scoped: only requested services that the daemon knows about.
	//    Not-running services in scope are simply started by `start`.
	if req.Restart {
		for _, name := range scope {
			if _, running := rtHashes[name]; !running {
				continue
			}
			tg.stop = append(tg.stop, name)
			tg.start = append(tg.start, name)
		}
	}

	tg.stop = uniqueSorted(tg.stop)
	tg.start = uniqueStable(tg.start)
	return tg, nil
}

func resolveScope(ws workspace.Record, requested []values.ServiceName) ([]values.ServiceName, error) {
	if len(requested) == 0 {
		// Manifest iteration order is not part of its contract: normalize here
		// so the plan is stable. Clone first: slices.Sort mutates in place.
		names := slices.Clone(ws.Manifest.ServiceNames())
		slices.Sort(names)
		return names, nil
	}

	seen := make(map[values.ServiceName]struct{}, len(requested))
	scope := make([]values.ServiceName, 0, len(requested))
	for _, n := range requested {
		if _, dup := seen[n]; dup {
			continue
		}
		if _, err := ws.Manifest.Get(n); err != nil {
			return nil, err
		}
		seen[n] = struct{}{}
		scope = append(scope, n) // preserves user order => deterministic first-error reporting
	}
	return scope, nil
}

func uniqueStable[S ~[]E, E comparable](s S) S {
	seen := make(map[E]struct{}, len(s))
	out := make(S, 0, len(s))
	for _, v := range s {
		if _, dup := seen[v]; dup {
			continue
		}
		seen[v] = struct{}{}
		out = append(out, v)
	}
	return out
}

func uniqueSorted[S ~[]E, E cmp.Ordered](s S) S {
	slices.Sort(s)
	return slices.Compact(s)
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

func BuildUpPhases(wsID values.WorkspaceID, ws workspace.Record, tg Targets) ([]Phase, error) {
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
