package hashing

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"sort"
)

// Hash is the single content-hash type used everywhere in the system —
// for a single service, for a whole manifest, and for the combined hash
// of currently running services. One type instead of ad hoc [32]byte in
// one place and string in another eliminates hex-case mismatches and
// "comparing incompatible representations" bugs at every call site.
type Hash [32]byte

func (h Hash) String() string { return hex.EncodeToString(h[:]) }
func (h Hash) IsZero() bool   { return h == Hash{} }

// NamedHash pairs a stable name with its hash, the unit Combine folds over.
type NamedHash struct {
	Name string
	Hash Hash
}

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
func Combine(pairs []NamedHash) Hash {
	sorted := append([]NamedHash(nil), pairs...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Name < sorted[j].Name })

	h := sha256.New()
	for _, p := range sorted {
		_, err := fmt.Fprintf(h, "name=%s\x00hash=%s\x00", p.Name, p.Hash)
		if err != nil {
			panic(err)
		}
	}

	var out Hash
	copy(out[:], h.Sum(nil))
	return out
}
