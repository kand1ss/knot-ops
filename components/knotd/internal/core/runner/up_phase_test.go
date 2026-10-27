// Package runner unit tests for up-phase planning.
//
// Covered scenarios:
//   - ResolveUpTargets: explicit and implicit start targets, pruning enabled/disabled,
//     empty runtime state, and unknown requested services.
//   - BuildPrunePhase: empty input, deterministic ordering, a single concurrent wave,
//     zero specs for stopped services, and input-slice immutability.
//   - BuildUpPhases: prune-only, start-only, combined prune-before-start plans,
//     dependency ordering, empty plans, and invalid start targets.
//   - Phase invariants: start units carry manifest specs and task IDs are non-empty
//     and unique within the generated plan.
package runner

import (
	"errors"
	"reflect"
	"slices"
	"testing"

	"github.com/google/uuid"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/requests"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

func testName(s string) values.ServiceName { return values.ServiceName(s) }

func testWorkspaceID(s uuid.UUID) values.WorkspaceId { return values.WorkspaceId(s) }

func testSpec(name string, depends ...string) domain.ServiceSpec {
	spec := domain.ServiceSpec{
		Name:      testName(name),
		Command:   "run-" + name,
		Directory: "/srv/" + name,
		Env:       map[string]string{"SERVICE": name},
	}
	for _, dep := range depends {
		depName := testName(dep)
		spec.Depends = append(spec.Depends, depName)
	}
	return spec
}

func testRecord(specs ...domain.ServiceSpec) workspace.Record {
	manifest := domain.NewWorkspaceManifest(specs...)
	return workspace.Record{
		Manifest: manifest,
		ServiceHashes: func() map[values.ServiceName]hashing.Hash {
			hashes := make(map[values.ServiceName]hashing.Hash, len(specs))
			for i, spec := range specs {
				var h hashing.Hash
				h[0] = byte(i + 1)
				hashes[spec.Name] = h
			}
			return hashes
		}(),
	}
}

func namesOf(units []PhaseUnit) []values.ServiceName {
	names := make([]values.ServiceName, 0, len(units))
	for _, unit := range units {
		names = append(names, unit.Service)
	}
	return names
}

func sortedNames(names []values.ServiceName) []values.ServiceName {
	out := slices.Clone(names)
	slices.Sort(out)
	return out
}

func assertNames(t *testing.T, got, want []values.ServiceName) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("service names = %v, want %v", got, want)
	}
}

func assertUniqueNonZeroTaskIDs(t *testing.T, phases []Phase) {
	t.Helper()

	seen := make(map[values.TaskID]struct{})

	for _, phase := range phases {
		for _, wave := range phase.Waves {
			for _, unit := range wave {
				if unit.Id == (values.TaskID{}) {
					t.Fatalf("service %q has a zero task ID", unit.Service)
				}

				if _, exists := seen[unit.Id]; exists {
					t.Fatalf("duplicate task ID %v", unit.Id)
				}

				seen[unit.Id] = struct{}{}
			}
		}
	}
}

func TestResolveUpTargets(t *testing.T) {
	wsID := testWorkspaceID(uuid.New())
	ws := testRecord(testSpec("api"), testSpec("db"), testSpec("worker"))

	var runtimeAPI, runtimeOrphan hashing.Hash
	runtimeAPI[0] = 1
	runtimeOrphan[0] = 9

	tests := []struct {
		name      string
		runtime   map[values.ServiceName]hashing.Hash
		request   requests.UpRequest
		wantStart []values.ServiceName
		wantStop  []values.ServiceName
		wantErr   bool
	}{
		{
			name:      "empty service list starts all manifest services",
			wantStart: []values.ServiceName{"api", "db", "worker"},
		},
		{
			name:      "explicit services preserve request order",
			request:   requests.UpRequest{Services: []values.ServiceName{"worker", "api"}},
			wantStart: []values.ServiceName{"worker", "api"},
		},
		{
			name:      "prune finds runtime-only services",
			runtime:   map[values.ServiceName]hashing.Hash{"api": runtimeAPI, "orphan": runtimeOrphan},
			request:   requests.UpRequest{Prune: true, Services: []values.ServiceName{"api"}},
			wantStart: []values.ServiceName{"api"},
			wantStop:  []values.ServiceName{"orphan"},
		},
		{
			name:      "prune disabled does not stop runtime-only services",
			runtime:   map[values.ServiceName]hashing.Hash{"orphan": runtimeOrphan},
			request:   requests.UpRequest{Services: []values.ServiceName{"api"}},
			wantStart: []values.ServiceName{"api"},
		},
		{
			name:    "unknown requested service returns typed error and empty targets",
			request: requests.UpRequest{Services: []values.ServiceName{"missing"}},
			wantErr: true,
		},
		{
			name:      "empty manifest and empty request produce empty targets",
			request:   requests.UpRequest{},
			wantStart: []values.ServiceName{},
		},
		{
			name:      "empty runtime state is safe when pruning",
			request:   requests.UpRequest{Prune: true, Services: []values.ServiceName{"api"}},
			wantStart: []values.ServiceName{"api"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			record := ws
			if tt.name == "empty manifest and empty request produce empty targets" {
				record = testRecord()
			}
			got, err := ResolveUpTargets(wsID, record, tt.runtime, tt.request)
			if tt.wantErr {
				if err == nil {
					t.Fatal("expected an error, got nil")
				}
				var missing ErrServiceNotExists
				if !errors.As(err, &missing) {
					t.Fatalf("error type = %T, want ErrServiceNotExists", err)
				}
				if missing.WorkspaceID != wsID || missing.Service != "missing" {
					t.Fatalf("error fields = (%q, %q), want (%q, %q)",
						missing.WorkspaceID, missing.Service, wsID, "missing")
				}
				if len(got.Start) != 0 || len(got.Stop) != 0 {
					t.Fatalf("targets on error = %+v, want empty targets", got)
				}
				return
			}
			if err != nil {
				t.Fatalf("ResolveUpTargets() error = %v", err)
			}

			gotStart := got.Start
			if len(tt.request.Services) == 0 {
				gotStart = sortedNames(gotStart)
			}
			wantStart := tt.wantStart
			if len(tt.request.Services) == 0 {
				wantStart = sortedNames(wantStart)
			}
			assertNames(t, gotStart, wantStart)
			assertNames(t, got.Stop, tt.wantStop)
		})
	}
}

func TestBuildPrunePhase(t *testing.T) {
	wsID := testWorkspaceID(uuid.New())
	ws := testRecord(testSpec("declared"))

	tests := []struct {
		name    string
		orphans []values.ServiceName
		want    []values.ServiceName
		wantNil bool
	}{
		{name: "empty orphan list returns nil phases", wantNil: true},
		{
			name:    "orphans are sorted into one wave",
			orphans: []values.ServiceName{"zeta", "alpha", "mu"},
			want:    []values.ServiceName{"alpha", "mu", "zeta"},
		},
		{
			name:    "single orphan",
			orphans: []values.ServiceName{"orphan"},
			want:    []values.ServiceName{"orphan"},
		},
		{
			name:    "duplicate names are preserved but sorted",
			orphans: []values.ServiceName{"beta", "alpha", "beta"},
			want:    []values.ServiceName{"alpha", "beta", "beta"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := slices.Clone(tt.orphans)
			got, err := BuildPrunePhase(wsID, ws, input)
			if err != nil {
				t.Fatalf("BuildPrunePhase() error = %v", err)
			}
			if !reflect.DeepEqual(input, tt.orphans) {
				t.Fatalf("input mutated: got %v, original %v", input, tt.orphans)
			}
			if tt.wantNil {
				if got != nil {
					t.Fatalf("phases = %#v, want nil", got)
				}
				return
			}
			if len(got) != 1 {
				t.Fatalf("phase count = %d, want 1", len(got))
			}
			phase := got[0]
			if phase.Kind != PhaseKindStop {
				t.Fatalf("phase kind = %v, want PhaseKindStop", phase.Kind)
			}
			if len(phase.Waves) != 1 {
				t.Fatalf("wave count = %d, want 1", len(phase.Waves))
			}
			assertNames(t, namesOf(phase.Waves[0]), tt.want)
			for _, unit := range phase.Waves[0] {
				if !reflect.DeepEqual(unit.Spec, domain.ServiceSpec{}) {
					t.Errorf("stop unit %q has spec %+v, want zero spec", unit.Service, unit.Spec)
				}
			}
			assertUniqueNonZeroTaskIDs(t, got)
		})
	}
}

func TestBuildUpPhases(t *testing.T) {
	wsID := testWorkspaceID(uuid.New())
	ws := testRecord(
		testSpec("db"),
		testSpec("api", "db"),
		testSpec("worker"),
	)

	t.Run("empty targets produce no phases", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 0 {
			t.Fatalf("phase count = %d, want 0", len(got))
		}
	})

	t.Run("prune-only plan contains stop phase", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{Stop: []values.ServiceName{"orphan"}})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 1 || got[0].Kind != PhaseKindStop {
			t.Fatalf("phases = %+v, want one stop phase", got)
		}
		assertNames(t, namesOf(got[0].Waves[0]), []values.ServiceName{"orphan"})
	})

	t.Run("start-only plan follows dependency order and copies specs", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{Start: []values.ServiceName{"api"}})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 1 || got[0].Kind != PhaseKindStart {
			t.Fatalf("phases = %+v, want one start phase", got)
		}
		phase := got[0]
		if len(phase.Waves) != 2 {
			t.Fatalf("wave count = %d, want 2 (db then api)", len(phase.Waves))
		}
		assertNames(t, namesOf(phase.Waves[0]), []values.ServiceName{"db"})
		assertNames(t, namesOf(phase.Waves[1]), []values.ServiceName{"api"})
		spec, ok := ws.Manifest.Get(testName("api"))
		if !ok {
			t.Fatal("test fixture missing api spec")
		}
		if !reflect.DeepEqual(phase.Waves[1][0].Spec, spec) {
			t.Fatalf("api spec = %+v, want manifest spec %+v", phase.Waves[1][0].Spec, spec)
		}
		assertUniqueNonZeroTaskIDs(t, got)
	})

	t.Run("prune phase always precedes start phase", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{
			Stop:  []values.ServiceName{"z-orphan", "a-orphan"},
			Start: []values.ServiceName{"api"},
		})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 2 {
			t.Fatalf("phase count = %d, want 2", len(got))
		}
		if got[0].Kind != PhaseKindStop || got[1].Kind != PhaseKindStart {
			t.Fatalf("phase kinds = (%v, %v), want (Stop, Start)", got[0].Kind, got[1].Kind)
		}
		assertNames(t, namesOf(got[0].Waves[0]), []values.ServiceName{"a-orphan", "z-orphan"})
		assertUniqueNonZeroTaskIDs(t, got)
	})

	t.Run("independent services may share a wave", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{Start: []values.ServiceName{"worker", "db"}})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 1 || len(got[0].Waves) == 0 {
			t.Fatalf("phases = %+v, want one non-empty start phase", got)
		}
		// The graph implementation determines ordering within a wave; assert the
		// membership without making the test depend on map iteration order.
		assertNames(t, sortedNames(namesOf(got[0].Waves[0])), []values.ServiceName{"db", "worker"})
	})

	t.Run("unknown start target returns error", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{Start: []values.ServiceName{"missing"}})
		if err == nil {
			t.Fatalf("BuildUpPhases() error = nil, phases = %+v", got)
		}
	})

	t.Run("empty start with empty prune list returns no phases", func(t *testing.T) {
		got, err := BuildUpPhases(wsID, ws, Targets{Start: []values.ServiceName{}})
		if err != nil {
			t.Fatalf("BuildUpPhases() error = %v", err)
		}
		if len(got) != 0 {
			t.Fatalf("phase count = %d, want 0", len(got))
		}
	})
}
