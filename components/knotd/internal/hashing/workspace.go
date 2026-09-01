package hashing

import (
	"crypto/sha256"
	"fmt"
	"sort"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

// CanonicalManifestHash builds the whole-workspace hash from per-service
// hashes, keyed by name and sorted, so declaration order in TOML doesn't
// matter but the actual set+content of services does. This replaces the
// duplicated inline logic from before — ServiceHash is now the single
// source of per-service canonicalization.
func CanonicalManifestHash(manifest domain.WorkspaceManifest) string {
	services := append([]domain.ServiceSpec(nil), manifest.Services()...)
	sort.Slice(services, func(i, j int) bool { return services[i].Name < services[j].Name })

	h := sha256.New()
	for _, svc := range services {
		svcHash := ServiceHash(svc)
		fmt.Fprintf(h, "name=%s\x00hash=%x\x00", svc.Name, svcHash)
	}

	return fmt.Sprintf("%x", h.Sum(nil))
}
