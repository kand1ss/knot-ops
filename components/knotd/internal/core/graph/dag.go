package graph

import (
	"errors"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

var ErrCycleDetected = errors.New("cycle detected in dependency graph")

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
