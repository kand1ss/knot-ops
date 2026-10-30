// Package runner unit tests for down-phase planning.
//
// Covered scenarios:
//   - Explicit stop targets include the requested services and their transitive dependents.
//   - Services that only the requested service depends on are not stopped.
//   - Reverse dependency ordering stops dependents before their dependencies.
//   - Multiple targets with overlapping dependent closures do not duplicate services.
//   - An empty request stops every service in the workspace.
//   - Unknown targets return ErrServiceNotExists without producing phases.
//   - Empty workspaces, independent services, stop-phase metadata, zero specs, and task IDs.
package phase

import (
	"errors"
	"reflect"
	"slices"
	"testing"

	"github.com/google/uuid"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

func TestBuildDownPhase(t *testing.T) {
	wsID := testWorkspaceID(uuid.New())
	// frontend -> api -> db; worker is independent.
	ws := testRecord(
		testSpec("db"),
		testSpec("api", "db"),
		testSpec("frontend", "api"),
		testSpec("worker"),
	)

	t.Run("requested service and transitive dependents are stopped", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{"api"})
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 {
			t.Fatalf("phase count = %d, want 1", len(got))
		}

		phase := got[0]
		if phase.kind != phaseKindStop {
			t.Fatalf("phase kind = %v, want PhaseKindStop", phase.kind)
		}
		if len(phase.waves) != 2 {
			t.Fatalf("wave count = %d, want 2 (frontend then api)", len(phase.waves))
		}
		assertNames(t, sortedNames(namesOf(phase.waves[0])), []values.ServiceName{"frontend"})
		assertNames(t, sortedNames(namesOf(phase.waves[1])), []values.ServiceName{"api"})

		// db is a dependency of api, not a dependent of api; worker is unrelated.
		for _, wave := range phase.waves {
			for _, unit := range wave {
				if unit.service == "db" || unit.service == "worker" {
					t.Errorf("unexpected service %q in stop plan", unit.service)
				}
			}
		}
		assertStopPhaseInvariants(t, got)
	})

	t.Run("transitive dependents are ordered before the requested dependency", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{"db"})
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 {
			t.Fatalf("phase count = %d, want 1", len(got))
		}
		phase := got[0]
		if len(phase.waves) != 3 {
			t.Fatalf("wave count = %d, want 3 (frontend, api, db)", len(phase.waves))
		}
		assertNames(t, sortedNames(namesOf(phase.waves[0])), []values.ServiceName{"frontend"})
		assertNames(t, sortedNames(namesOf(phase.waves[1])), []values.ServiceName{"api"})
		assertNames(t, sortedNames(namesOf(phase.waves[2])), []values.ServiceName{"db"})
		assertStopPhaseInvariants(t, got)
	})

	t.Run("overlapping targets do not duplicate services", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{"api", "db"})
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 {
			t.Fatalf("phase count = %d, want 1", len(got))
		}
		var all []values.ServiceName
		for _, wave := range got[0].waves {
			all = append(all, namesOf(wave)...)
		}
		if !reflect.DeepEqual(sortedNames(all), []values.ServiceName{"api", "db", "frontend"}) {
			t.Fatalf("stopped services = %v, want [api db frontend]", sortedNames(all))
		}
		if len(all) != len(slices.Compact(sortedNames(all))) {
			t.Fatalf("stop plan contains duplicate services: %v", all)
		}
		assertStopPhaseInvariants(t, got)
	})

	t.Run("empty request stops all manifest services", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, nil)
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 {
			t.Fatalf("phase count = %d, want 1", len(got))
		}

		var all []values.ServiceName
		for _, wave := range got[0].waves {
			all = append(all, namesOf(wave)...)
		}
		assertNames(t, sortedNames(all), []values.ServiceName{"api", "db", "frontend", "worker"})
		if len(all) != len(slices.Compact(sortedNames(all))) {
			t.Fatalf("stop plan contains duplicate services: %v", all)
		}
		assertStopPhaseInvariants(t, got)
	})

	t.Run("empty request also stops all services when passed as empty slice", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{})
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		var all []values.ServiceName
		for _, phase := range got {
			for _, wave := range phase.waves {
				all = append(all, namesOf(wave)...)
			}
		}
		assertNames(t, sortedNames(all), []values.ServiceName{"api", "db", "frontend", "worker"})
	})

	t.Run("independent requested services can share a wave", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{"worker"})
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 || len(got[0].waves) != 1 {
			t.Fatalf("phases = %+v, want one phase with one wave", got)
		}
		assertNames(t, namesOf(got[0].waves[0]), []values.ServiceName{"worker"})
		assertStopPhaseInvariants(t, got)
	})

	t.Run("unknown requested service returns typed error and no phases", func(t *testing.T) {
		got, err := BuildDownPhase(values.NewTaskID(), wsID, ws, []values.ServiceName{"missing"})
		if err == nil {
			t.Fatalf("BuildDownPhase() error = nil, phases = %+v", got)
		}
		var missing domain.ErrServiceNotInManifest
		if !errors.As(err, &missing) {
			t.Fatalf("error type = %T, want ErrServiceNotExists", err)
		}
		if missing.Service != "missing" {
			t.Fatalf("error fields = %q, want %q",
				missing.Service, "missing")
		}
		if got != nil {
			t.Fatalf("phases = %+v, want nil on validation error", got)
		}
	})

	t.Run("empty manifest produces an empty stop plan", func(t *testing.T) {
		emptyWS := testRecord()
		got, err := BuildDownPhase(values.NewTaskID(), wsID, emptyWS, nil)
		if err != nil {
			t.Fatalf("BuildDownPhase() error = %v", err)
		}
		if len(got) != 1 {
			t.Fatalf("phase count = %d, want 1 stop phase", len(got))
		}
		if got[0].kind != phaseKindStop {
			t.Fatalf("phase kind = %v, want PhaseKindStop", got[0].kind)
		}
		if len(got[0].waves) != 0 {
			t.Fatalf("wave count = %d, want 0 for an empty manifest", len(got[0].waves))
		}
		assertStopPhaseInvariants(t, got)
	})
}

func assertStopPhaseInvariants(t *testing.T, phases []Phase) {
	t.Helper()

	for _, phase := range phases {
		if phase.kind != phaseKindStop {
			t.Errorf("phase kind = %v, want PhaseKindStop", phase.kind)
		}
		for _, wave := range phase.waves {
			for _, unit := range wave {
				if !reflect.DeepEqual(unit.spec, domain.ServiceSpec{}) {
					t.Errorf("stop unit %q has spec %+v, want zero spec", unit.spec.Name, unit.spec)
				}
			}
		}
	}
	assertUniqueNonZeroTaskIDs(t, phases)
}
