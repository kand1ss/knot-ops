package hashing

import (
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

// Helper function to build WorkspaceManifest with given services.2*
func makeManifest(services ...domain.ServiceSpec) domain.WorkspaceManifest {
	var m domain.WorkspaceManifest
	for _, s := range services {
		m.Append(s)
	}
	return m
}

// 1. Consistency: Repeated calls on the same manifest or identical manifests return the same hash.
func TestCanonicalManifestHash_Consistency(t *testing.T) {
	manifest1 := makeManifest(baseService("api"), baseService("worker"))
	manifest2 := makeManifest(baseService("api"), baseService("worker"))

	hash1 := CanonicalManifestHash(manifest1)
	hash2 := CanonicalManifestHash(manifest1) // Repeated call on the exact same instance
	hash3 := CanonicalManifestHash(manifest2) // Call on an identical manifest

	if hash1 != hash2 {
		t.Errorf("repeated call returned a different hash: %s != %s", hash1, hash2)
	}
	if hash1 != hash3 {
		t.Errorf("identical manifests returned different hashes: %s != %s", hash1, hash3)
	}
}

// 3. Workspace hash calculation: Service hash aggregation, order independence, and field/name propagation.
func TestCanonicalManifestHash_WorkspaceAggregation(t *testing.T) {
	svcA := baseService("auth")
	svcB := baseService("billing")
	svcB.Command = "node index.js"

	t.Run("Service declaration order does not affect workspace hash", func(t *testing.T) {
		manifestOrder1 := makeManifest(svcA, svcB)
		manifestOrder2 := makeManifest(svcB, svcA)

		hash1 := CanonicalManifestHash(manifestOrder1)
		hash2 := CanonicalManifestHash(manifestOrder2)

		if hash1 != hash2 {
			t.Errorf("declaration order changed workspace hash: %s != %s", hash1, hash2)
		}
	})

	t.Run("Modifying a service field changes workspace hash", func(t *testing.T) {
		manifestOriginal := makeManifest(svcA, svcB)

		svcBModified := copyService(svcB)
		svcBModified.Command = "node server.js"
		manifestModified := makeManifest(svcA, svcBModified)

		hash1 := CanonicalManifestHash(manifestOriginal)
		hash2 := CanonicalManifestHash(manifestModified)

		if hash1 == hash2 {
			t.Error("modifying inner service field did not change workspace hash")
		}
	})

	t.Run("Changing a service name changes workspace hash", func(t *testing.T) {
		manifest1 := makeManifest(svcA, svcB)

		svcARenamed := copyService(svcA)
		svcARenamed.Name = "auth-v2"
		manifest2 := makeManifest(svcARenamed, svcB)

		hash1 := CanonicalManifestHash(manifest1)
		hash2 := CanonicalManifestHash(manifest2)

		if hash1 == hash2 {
			t.Error("changing service name did not change workspace hash")
		}
	})
}
