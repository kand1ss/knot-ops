package diff

import (
	"reflect"
	"slices"
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

func TestComputeDiff(t *testing.T) {
	hashA := hashing.Hash{1}
	hashA2 := hashing.Hash{1, 1}
	hashB := hashing.Hash{2}
	hashC := hashing.Hash{3}
	zeroHash := hashing.Hash{}

	tests := []struct {
		name     string
		previous map[values.ServiceName]hashing.Hash
		incoming map[values.ServiceName]hashing.Hash
		expected Diff
	}{
		{
			name:     "Both maps empty",
			previous: map[values.ServiceName]hashing.Hash{},
			incoming: map[values.ServiceName]hashing.Hash{},
			expected: Diff{},
		},
		{
			name:     "Both maps nil",
			previous: nil,
			incoming: nil,
			expected: Diff{},
		},
		{
			name:     "Nil previous map, non-empty incoming",
			previous: nil,
			incoming: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
			},
			expected: Diff{
				Added: []values.ServiceName{"svc-1"},
			},
		},
		{
			name: "Non-empty previous map, nil incoming",
			previous: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
			},
			incoming: nil,
			expected: Diff{
				Removed: []values.ServiceName{"svc-1"},
			},
		},
		{
			name: "No changes",
			previous: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
				"svc-2": hashB,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
				"svc-2": hashB,
			},
			expected: Diff{},
		},
		{
			name: "Only added services",
			previous: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
				"svc-2": hashB,
				"svc-3": hashC,
			},
			expected: Diff{
				Added: []values.ServiceName{"svc-2", "svc-3"},
			},
		},
		{
			name: "Only removed services",
			previous: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
				"svc-2": hashB,
				"svc-3": hashC,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
			},
			expected: Diff{
				Removed: []values.ServiceName{"svc-2", "svc-3"},
			},
		},
		{
			name: "Only updated services",
			previous: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA,
				"svc-2": hashB,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"svc-1": hashA2,
				"svc-2": hashC,
			},
			expected: Diff{
				Updated: []values.ServiceName{"svc-1", "svc-2"},
			},
		},
		{
			name: "Mixed changes (Added, Removed, Updated, Unchanged)",
			previous: map[values.ServiceName]hashing.Hash{
				"unchanged": hashA,
				"updated":   hashB,
				"removed":   hashC,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"unchanged": hashA,
				"updated":   hashA2,
				"added":     hashC,
			},
			expected: Diff{
				Added:   []values.ServiceName{"added"},
				Removed: []values.ServiceName{"removed"},
				Updated: []values.ServiceName{"updated"},
			},
		},
		{
			name: "Handling zero-value hashes",
			previous: map[values.ServiceName]hashing.Hash{
				"zero-unchanged":  zeroHash,
				"zero-to-nonzero": zeroHash,
			},
			incoming: map[values.ServiceName]hashing.Hash{
				"zero-unchanged":  zeroHash,
				"zero-to-nonzero": hashA,
			},
			expected: Diff{
				Updated: []values.ServiceName{"zero-to-nonzero"},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			actual := ComputeDiff(tt.previous, tt.incoming)

			// Go map iteration is non-deterministic, so order in result slices varies.
			// Sort slices before comparing.
			normalizeDiff(&actual)
			normalizeDiff(&tt.expected)

			if !reflect.DeepEqual(tt.expected, actual) {
				t.Errorf("ComputeDiff mismatch:\nexpected: %+v\nactual:   %+v", tt.expected, actual)
			}
		})
	}
}

func normalizeDiff(d *Diff) {
	slices.Sort(d.Added)
	slices.Sort(d.Removed)
	slices.Sort(d.Updated)
}
