package hashing

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"slices"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// Hash is the single content-hash type used everywhere in the system —
// for a single service, for a whole manifest, and for the combined hash
// of currently running services. One type instead of ad hoc [32]byte in
// one place and string in another eliminates hex-case mismatches and
// "comparing incompatible representations" bugs at every call site.
type Hash [32]byte

func (h Hash) String() string { return hex.EncodeToString(h[:]) }
func (h Hash) IsZero() bool   { return h == Hash{} }

// Combine is the single canonical algorithm for folding a set of named
// hashes into one combined Hash. It is used both when building a
// WorkspaceRecord from declared ServiceSpecs (workspace.BuildWorkspaceRecord)
// and when computing the "currently running" hash from live ServiceHandles
// (runstate's WorkspaceHash/Snapshot). Both callers MUST go through this
// one function — if they ever diverge (e.g. one sorts, one doesn't), "in
// sync" comparisons between declared and running state become meaningless
// by construction, and no amount of testing the individual callers catches it.
//
// Assumes names are unique within pairs — duplicate service names are a
// manifest validation error that must be rejected upstream (config
// parsing), not a concern Combine re-defends against here.
func Combine(m map[values.ServiceName]Hash) Hash {
	names := make([]values.ServiceName, 0, len(m))
	for name := range m {
		names = append(names, name)
	}

	slices.Sort(names)

	h := sha256.New()
	for _, name := range names {
		hash := m[name]
		_, err := fmt.Fprintf(h, "name=%d:%s\x00hash=%s\x00", len(name), name, hash)
		if err != nil {
			panic(err)
		}
	}

	var out Hash
	copy(out[:], h.Sum(nil))
	return out
}
