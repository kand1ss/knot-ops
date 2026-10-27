package graph

import (
	"errors"
	"fmt"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

var ErrCycleDetected = errors.New("cycle detected in dependency graph")
var ErrNodeNotFound = errors.New("node not found in dependency graph")

type DependencyGraph[T comparable] struct {
	adj   map[T][]T
	nodes map[T]struct{}
}

func NewDependencyGraph[T comparable]() DependencyGraph[T] {
	return DependencyGraph[T]{
		adj:   make(map[T][]T),
		nodes: make(map[T]struct{}),
	}
}

func NewDependencyGraphFromServices(services []domain.ServiceSpec) DependencyGraph[values.ServiceName] {
	graph := NewDependencyGraph[values.ServiceName]()

	for _, service := range services {
		graph.AddNode(service.Name, service.Depends...)
	}
	return graph
}

func (g *DependencyGraph[T]) AddNode(node T, depends ...T) {
	g.nodes[node] = struct{}{}
	for _, dep := range depends {
		g.nodes[dep] = struct{}{}
		g.adj[node] = append(g.adj[node], dep)
	}
}

func (g *DependencyGraph[T]) subgraph(targets ...T) (DependencyGraph[T], error) {
	requiredNodes := make(map[T]struct{})

	var collectDeps func(node T) error
	collectDeps = func(node T) error {
		if _, exists := g.nodes[node]; !exists {
			return fmt.Errorf("%w: %v", ErrNodeNotFound, node)
		}
		if _, visited := requiredNodes[node]; visited {
			return nil
		}

		requiredNodes[node] = struct{}{}
		for _, dep := range g.adj[node] {
			if err := collectDeps(dep); err != nil {
				return err
			}
		}
		return nil
	}

	for _, target := range targets {
		if err := collectDeps(target); err != nil {
			return DependencyGraph[T]{}, err
		}
	}

	sub := NewDependencyGraph[T]()
	for node := range requiredNodes {
		sub.nodes[node] = struct{}{}
		for _, dep := range g.adj[node] {
			sub.adj[node] = append(sub.adj[node], dep)
			sub.nodes[dep] = struct{}{}
		}
	}

	return sub, nil
}

// dependentsClosure returns the subgraph induced by targets plus every node
// that transitively depends on any of them, with all edges TRANSPOSED:
// in the result, `dependency -> dependent` becomes `dependent` being a
// prerequisite of `dependency`. Running BuildWaves on it therefore yields
// dependents-first (stop) order, with cycle detection for free.
//
// Dependencies of the targets are deliberately NOT included: stopping
// `backend` must not stop `postgres`.
func (g *DependencyGraph[T]) dependentsClosure(targets ...T) (DependencyGraph[T], error) {
	// Reverse adjacency over the full graph: dep -> nodes that depend on it.
	dependents := make(map[T][]T, len(g.nodes))
	for node, deps := range g.adj {
		for _, dep := range deps {
			dependents[dep] = append(dependents[dep], node)
		}
	}

	// Iterative DFS: no recursion depth risk on long chains.
	closure := make(map[T]struct{}, len(targets))
	stack := make([]T, 0, len(targets))
	for _, t := range targets {
		if _, ok := g.nodes[t]; !ok {
			return DependencyGraph[T]{}, fmt.Errorf("%w: %v", ErrNodeNotFound, t)
		}
		if _, seen := closure[t]; !seen {
			closure[t] = struct{}{}
			stack = append(stack, t)
		}
	}
	for len(stack) > 0 {
		n := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		for _, d := range dependents[n] {
			if _, seen := closure[d]; !seen {
				closure[d] = struct{}{}
				stack = append(stack, d)
			}
		}
	}

	// Build the transposed induced subgraph. Edges leaving the closure
	// (dependencies of the targets) are dropped on purpose.
	rev := NewDependencyGraph[T]()
	for node := range closure {
		rev.nodes[node] = struct{}{}
	}
	for node := range closure {
		for _, dep := range g.adj[node] {
			if _, in := closure[dep]; in {
				// original: node depends on dep
				// reversed: dep waits for node (node is stopped first)
				rev.adj[dep] = append(rev.adj[dep], node)
			}
		}
	}
	return rev, nil
}

// BuildRevertedWavesFor returns waves in STOP order for the given targets and
// everything that transitively depends on them. Wave i contains only nodes
// whose in-scope dependents all sit in waves < i, so nodes inside a wave can be
// stopped concurrently and a wave must complete before the next one starts.
//
// Unlike reversing BuildWavesFor, dependencies of the targets are excluded
// from the result and parallelism is maximal (levels are measured from the
// dependents side).
func (g *DependencyGraph[T]) BuildRevertedWavesFor(targets ...T) ([][]T, error) {
	if len(targets) == 0 {
		return nil, nil
	}
	rev, err := g.dependentsClosure(targets...)
	if err != nil {
		return nil, err
	}
	return rev.BuildWaves()
}

func (g *DependencyGraph[T]) BuildWavesFor(targets ...T) ([][]T, error) {
	if len(targets) == 0 {
		return nil, nil
	}

	sub, err := g.subgraph(targets...)
	if err != nil {
		return nil, err
	}

	return sub.BuildWaves()
}

func (g *DependencyGraph[T]) BuildWaves() ([][]T, error) {
	inDegree := make(map[T]int, len(g.nodes))
	dependents := make(map[T][]T)

	for node := range g.nodes {
		inDegree[node] = 0
	}

	for node, deps := range g.adj {
		inDegree[node] = len(deps)
		for _, dep := range deps {
			dependents[dep] = append(dependents[dep], node)
		}
	}

	var currWave []T
	for node, count := range inDegree {
		if count == 0 {
			currWave = append(currWave, node)
		}
	}

	var waves [][]T
	processedCount := 0

	for len(currWave) > 0 {
		waves = append(waves, currWave)
		processedCount += len(currWave)

		var nextWave []T
		for _, node := range currWave {
			for _, depNode := range dependents[node] {
				inDegree[depNode]--
				if inDegree[depNode] == 0 {
					nextWave = append(nextWave, depNode)
				}
			}
		}

		currWave = nextWave
	}

	if processedCount != len(g.nodes) {
		return nil, ErrCycleDetected
	}

	return waves, nil
}
