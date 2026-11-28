package workspace

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type Record struct {
	Manifest      domain.WorkspaceManifest
	Hash          hashing.Hash
	ServiceHashes map[values.ServiceName]hashing.Hash
}

func BuildWorkspaceRecord(manifest domain.WorkspaceManifest) (Record, error) {
	serviceHashes, err := hashing.ServicesHash(manifest.Services())
	if err != nil {
		return Record{}, err
	}

	canonicalHash := hashing.Combine(serviceHashes)
	return Record{
		Manifest:      manifest,
		Hash:          canonicalHash,
		ServiceHashes: serviceHashes,
	}, nil
}

func (r *Record) Equals(to Record) bool {
	return r.Hash == to.Hash
}
