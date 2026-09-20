package sync

import "github.com/kand1ss/knot-ops/components/knotd/internal/values"

type Diff struct {
	Added   []values.ServiceName
	Removed []values.ServiceName
	Updated []values.ServiceName
}

func ComputeDiff(previous map[values.ServiceName][32]byte, incoming map[values.ServiceName][32]byte) Diff {
	var diff Diff
	for name, newHash := range incoming {
		oldHash, ok := previous[name]
		switch {
		case !ok:
			diff.Added = append(diff.Added, name)
		case newHash != oldHash:
			diff.Updated = append(diff.Updated, name)
		}
	}

	for name := range previous {
		if _, ok := incoming[name]; !ok {
			diff.Removed = append(diff.Removed, name)
		}
	}

	return diff
}
