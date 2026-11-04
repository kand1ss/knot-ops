package hashing

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
)

// CanonicalManifestHash builds the whole-workspace hash from per-service
// hashes, keyed by name and sorted, so declaration order in TOML doesn't
// matter but the actual set+content of services does. This replaces the
// duplicated inline logic from before — ServiceHash is now the single
// source of per-service canonicalization.
func CanonicalManifestHash(manifest domain.WorkspaceManifest) (Hash, error) {
	pairs, err := ServicesHash(manifest.Services())
	if err != nil {
		return Hash{}, err
	}
	return Combine(pairs), nil
}
