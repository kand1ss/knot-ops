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
