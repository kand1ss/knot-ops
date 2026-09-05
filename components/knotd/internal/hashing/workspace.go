package hashing

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

// CanonicalManifestHash builds the whole-workspace hash from per-service
// hashes, keyed by name and sorted, so declaration order in TOML doesn't
// matter but the actual set+content of services does. This replaces the
// duplicated inline logic from before — ServiceHash is now the single
// source of per-service canonicalization.
func CanonicalManifestHash(manifest domain.WorkspaceManifest) (Hash, error) {
	pairs := make([]NamedHash, 0, len(manifest.Services()))
	for _, svc := range manifest.Services() {
		h, err := ServiceHash(svc)
		if err != nil {
			return Hash{}, err
		}
		pairs = append(pairs, NamedHash{Name: string(svc.Name), Hash: h})
	}
	return Combine(pairs), nil
}
