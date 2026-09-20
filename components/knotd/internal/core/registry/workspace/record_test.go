package workspace

import (
	"reflect"
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

func TestBuildWorkspaceRecord(t *testing.T) {
	t.Parallel()

	t.Run("happy path: valid manifest with services", func(t *testing.T) {
		t.Parallel()

		svc1 := domain.ServiceSpec{
			Name:      values.ServiceName("web"),
			Command:   "npm start",
			Directory: "/web",
			Env:       map[string]string{"NODE_ENV": "production"},
		}
		svc2 := domain.ServiceSpec{
			Name:      values.ServiceName("db"),
			Command:   "postgres",
			Directory: "/db",
			Depends:   []values.ServiceName{values.ServiceName("web")},
		}

		manifest := domain.NewWorkspaceManifest(svc1, svc2)

		record, err := BuildWorkspaceRecord(manifest)
		if err != nil {
			t.Fatalf("expected no error, got: %v", err)
		}

		if len(record.ServiceHashes) != 2 {
			t.Fatalf("expected 2 service hashes, got %d", len(record.ServiceHashes))
		}

		var zeroHash hashing.Hash
		if record.Hash == zeroHash {
			t.Error("expected non-zero manifest hash")
		}

		if hashWeb, ok := record.ServiceHashes[values.ServiceName("web")]; !ok || hashWeb == zeroHash {
			t.Errorf("invalid or missing hash for service 'web'")
		}
		if hashDB, ok := record.ServiceHashes[values.ServiceName("db")]; !ok || hashDB == zeroHash {
			t.Errorf("invalid or missing hash for service 'db'")
		}

		if !reflect.DeepEqual(record.Manifest, manifest) {
			t.Errorf("manifest mismatch in record")
		}
	})

	t.Run("edge case: empty workspace manifest", func(t *testing.T) {
		t.Parallel()

		manifest := domain.NewWorkspaceManifest()

		record, err := BuildWorkspaceRecord(manifest)
		if err != nil {
			t.Fatalf("expected no error for empty manifest, got: %v", err)
		}

		if len(record.ServiceHashes) != 0 {
			t.Errorf("expected empty ServiceHashes, got size %d", len(record.ServiceHashes))
		}

		var zeroHash hashing.Hash
		if record.Hash == zeroHash {
			t.Error("expected non-zero hash for empty manifest")
		}
	})

	t.Run("edge case: manifest ignore duplicates added via Append", func(t *testing.T) {
		t.Parallel()

		svc := domain.ServiceSpec{Name: values.ServiceName("api"), Command: "go run main.go"}
		manifest := domain.NewWorkspaceManifest(svc)

		manifest.Append(domain.ServiceSpec{Name: "api", Command: "different command"})

		record, err := BuildWorkspaceRecord(manifest)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}

		if len(record.ServiceHashes) != 1 {
			t.Errorf("expected 1 service hash due to Append deduplication, got %d", len(record.ServiceHashes))
		}
	})
}
