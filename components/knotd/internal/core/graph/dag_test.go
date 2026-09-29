package graph

import (
	"errors"
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

func equalUnordered[T comparable](a, b []T) bool {
	if len(a) != len(b) {
		return false
	}
	counts := make(map[T]int, len(a))
	for _, val := range a {
		counts[val]++
	}
	for _, val := range b {
		counts[val]--
		if counts[val] < 0 {
			return false
		}
	}
	return true
}

func assertWavesEqual[T comparable](t *testing.T, expected, actual [][]T) {
	t.Helper()
	if len(expected) != len(actual) {
		t.Fatalf("mismatch in waves count: expected %d, got %d. Actual: %v", len(expected), len(actual), actual)
	}
	for i := range expected {
		if !equalUnordered(expected[i], actual[i]) {
			t.Errorf("wave %d mismatch:\nexpected (unordered): %v\ngot:                 %v", i+1, expected[i], actual[i])
		}
	}
}

func TestBuildWaves_HappyPaths(t *testing.T) {
	tests := []struct {
		name     string
		build    func() DependencyGraph[string]
		expected [][]string
	}{
		{
			name: "Linear dependency chain (A -> B -> C)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("C", "B")
				g.AddNode("B", "A")
				return g
			},
			expected: [][]string{
				{"A"},
				{"B"},
				{"C"},
			},
		},
		{
			name: "Diamond graph structure",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("A", "B", "C")
				g.AddNode("B", "D")
				g.AddNode("C", "D")
				return g
			},
			expected: [][]string{
				{"D"},
				{"B", "C"},
				{"A"},
			},
		},
		{
			name: "Complex multi-branch DAG",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("Auth", "DB")
				g.AddNode("Storage", "Cache")
				g.AddNode("API", "Auth", "Storage")
				return g
			},
			expected: [][]string{
				{"DB", "Cache"},
				{"Auth", "Storage"},
				{"API"},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			g := tt.build()
			waves, err := g.BuildWaves()
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			assertWavesEqual(t, tt.expected, waves)
		})
	}
}

func TestBuildWaves_Cycles(t *testing.T) {
	tests := []struct {
		name  string
		build func() DependencyGraph[string]
	}{
		{
			name: "Self loop (A depends on A)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("A", "A")
				return g
			},
		},
		{
			name: "Direct 2-node cycle (A -> B -> A)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("A", "B")
				g.AddNode("B", "A")
				return g
			},
		},
		{
			name: "Indirect 3-node cycle (A -> B -> C -> A)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("A", "B")
				g.AddNode("B", "C")
				g.AddNode("C", "A")
				return g
			},
		},
		{
			name: "Partial cycle in subgraph (D -> E is valid, A -> B -> A is cyclic)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("E", "D")
				g.AddNode("A", "B")
				g.AddNode("B", "A")
				return g
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			g := tt.build()
			_, err := g.BuildWaves()
			if err == nil {
				t.Fatal("expected cycle error, got nil")
			}
			if !errors.Is(err, ErrCycleDetected) {
				t.Errorf("expected ErrCycleDetected, got %v", err)
			}
		})
	}
}

func TestBuildWaves_EdgeCases(t *testing.T) {
	t.Run("Empty graph", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		waves, err := g.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if len(waves) != 0 {
			t.Errorf("expected 0 waves for empty graph, got %d", len(waves))
		}
	})

	t.Run("Single isolated node", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A")

		expected := [][]string{{"A"}}
		waves, err := g.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})

	t.Run("Multiple isolated nodes without dependencies", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A")
		g.AddNode("B")
		g.AddNode("C")

		expected := [][]string{{"A", "B", "C"}}
		waves, err := g.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})

	t.Run("Duplicate AddNode calls", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A")
		g.AddNode("A")
		g.AddNode("B", "A")

		expected := [][]string{{"A"}, {"B"}}
		waves, err := g.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})

	t.Run("Generics support (int keys)", func(t *testing.T) {
		g := NewDependencyGraph[int]()
		g.AddNode(3, 2)
		g.AddNode(2, 1)

		expected := [][]int{{1}, {2}, {3}}
		waves, err := g.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})
}

func TestBuildWavesFor_HappyPaths(t *testing.T) {
	tests := []struct {
		name     string
		build    func() DependencyGraph[string]
		targets  []string
		expected [][]string
	}{
		{
			name: "Target subset in a linear chain (Target B in A -> B -> C)",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("C", "B")
				g.AddNode("B", "A")
				return g
			},
			targets: []string{"B"},
			expected: [][]string{
				{"A"},
				{"B"},
			},
		},
		{
			name: "Target subset excludes unrelated branches",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("Auth", "DB")
				g.AddNode("API", "Auth")
				g.AddNode("Storage", "Cache")
				g.AddNode("UI", "Storage")
				return g
			},
			targets: []string{"API"},
			expected: [][]string{
				{"DB"},
				{"Auth"},
				{"API"},
			},
		},
		{
			name: "Multiple targets with shared dependencies",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("D", "B", "C")
				g.AddNode("B", "A")
				g.AddNode("C", "A")
				return g
			},
			targets: []string{"B", "C"},
			expected: [][]string{
				{"A"},
				{"B", "C"},
			},
		},
		{
			name: "Multiple targets from distinct subgraphs",
			build: func() DependencyGraph[string] {
				g := NewDependencyGraph[string]()
				g.AddNode("ServiceA", "DB1")
				g.AddNode("ServiceB", "DB2")
				return g
			},
			targets: []string{"ServiceA", "ServiceB"},
			expected: [][]string{
				{"DB1", "DB2"},
				{"ServiceA", "ServiceB"},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			g := tt.build()
			waves, err := g.BuildWavesFor(tt.targets...)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			assertWavesEqual(t, tt.expected, waves)
		})
	}
}

func TestBuildWavesFor_Cycles(t *testing.T) {
	t.Run("Cycle inside target's dependency path", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("C", "B")
		g.AddNode("B", "A")
		g.AddNode("A", "B")

		_, err := g.BuildWavesFor("C")
		if err == nil {
			t.Fatal("expected ErrCycleDetected, got nil")
		}
		if !errors.Is(err, ErrCycleDetected) {
			t.Errorf("expected ErrCycleDetected, got %v", err)
		}
	})

	t.Run("Isolated cycle in graph does NOT affect target outside cycle", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("X", "Y")
		g.AddNode("Y", "X")

		g.AddNode("C", "B")
		g.AddNode("B", "A")

		expected := [][]string{
			{"A"},
			{"B"},
			{"C"},
		}

		waves, err := g.BuildWavesFor("C")
		if err != nil {
			t.Fatalf("unexpected error processing independent target: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})
}

func TestBuildWavesFor_EdgeCasesAndErrors(t *testing.T) {
	t.Run("No targets provided (empty arguments)", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A", "B")

		waves, err := g.BuildWavesFor()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if waves != nil {
			t.Errorf("expected nil waves for empty targets, got %v", waves)
		}
	})

	t.Run("Single isolated target without dependencies", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A")
		g.AddNode("B", "A")

		expected := [][]string{{"A"}}
		waves, err := g.BuildWavesFor("A")
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		assertWavesEqual(t, expected, waves)
	})

	t.Run("Non-existent target node returns error", func(t *testing.T) {
		g := NewDependencyGraph[string]()
		g.AddNode("A")

		_, err := g.BuildWavesFor("NonExistentNode")
		if err == nil {
			t.Fatal("expected error for non-existent node target, got nil")
		}
	})
}

func TestNewDependencyGraphFromServices(t *testing.T) {
	t.Run("Happy path: valid services slice", func(t *testing.T) {
		services := []domain.ServiceSpec{
			{
				Name:    "frontend",
				Depends: []values.ServiceName{"api"},
			},
			{
				Name:    "api",
				Depends: []values.ServiceName{"db", "cache"},
			},
			{
				Name:    "db",
				Depends: nil,
			},
			{
				Name:    "cache",
				Depends: nil,
			},
		}

		graph := NewDependencyGraphFromServices(services)
		waves, err := graph.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}

		expected := [][]values.ServiceName{
			{"db", "cache"},
			{"api"},
			{"frontend"},
		}

		assertWavesEqual(t, expected, waves)
	})

	t.Run("Implicit dependencies: service depends on unlisted node", func(t *testing.T) {
		services := []domain.ServiceSpec{
			{
				Name:    "worker",
				Depends: []values.ServiceName{"db"},
			},
		}

		graph := NewDependencyGraphFromServices(services)
		waves, err := graph.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}

		expected := [][]values.ServiceName{
			{"db"},
			{"worker"},
		}

		assertWavesEqual(t, expected, waves)
	})

	t.Run("Fail path: cyclic dependencies between services", func(t *testing.T) {
		services := []domain.ServiceSpec{
			{
				Name:    "auth-service",
				Depends: []values.ServiceName{"user-service"},
			},
			{
				Name:    "user-service",
				Depends: []values.ServiceName{"auth-service"},
			},
		}

		graph := NewDependencyGraphFromServices(services)
		_, err := graph.BuildWaves()
		if err == nil {
			t.Fatal("expected ErrCycleDetected, got nil")
		}
		if !errors.Is(err, ErrCycleDetected) {
			t.Errorf("expected ErrCycleDetected, got %v", err)
		}
	})

	t.Run("Edge case: empty services slice", func(t *testing.T) {
		graph := NewDependencyGraphFromServices(nil)
		waves, err := graph.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if len(waves) != 0 {
			t.Errorf("expected 0 waves, got %d", len(waves))
		}
	})

	t.Run("Edge case: services without dependencies", func(t *testing.T) {
		services := []domain.ServiceSpec{
			{Name: "service-a"},
			{Name: "service-b"},
		}

		graph := NewDependencyGraphFromServices(services)
		waves, err := graph.BuildWaves()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}

		expected := [][]values.ServiceName{
			{"service-a", "service-b"},
		}

		assertWavesEqual(t, expected, waves)
	})
}
