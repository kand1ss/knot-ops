package workspace

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

type Record struct {
	Manifest      domain.WorkspaceManifest
	Hash          hashing.Hash
	ServiceHashes map[values.ServiceName]hashing.Hash
}

func BuildWorkspaceRecord(manifest domain.WorkspaceManifest) (Record, error) {
	serviceHashes := make(map[values.ServiceName]hashing.Hash, len(manifest.Services()))
	for _, svc := range manifest.Services() {
		hash, err := hashing.ServiceHash(svc)
		if err != nil {
			return Record{}, err
		}
		serviceHashes[svc.Name] = hash
	}

	hash, err := hashing.CanonicalManifestHash(manifest)
	if err != nil {
		return Record{}, err
	}
	return Record{
		Manifest:      manifest,
		Hash:          hash,
		ServiceHashes: serviceHashes,
	}, nil
}
