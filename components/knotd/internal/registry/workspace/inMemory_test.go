package workspace

import (
	"maps"
	"reflect"
	"sync"
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

func assertRecordEqual(t *testing.T, expected, got Record) {
	t.Helper()

	if got.Hash != expected.Hash {
		t.Fatalf("hash mismatch: expected %v, got %v", expected.Hash, got.Hash)
	}

	if !maps.Equal(got.ServiceHashes, expected.ServiceHashes) {
		t.Fatalf("service hashes mismatch: expected %v, got %v", expected.ServiceHashes, got.ServiceHashes)
	}

	if !reflect.DeepEqual(got.Manifest, expected.Manifest) {
		t.Fatalf("manifest mismatch: expected %+v, got %+v", expected.Manifest, got.Manifest)
	}
}

func TestNewInMemoryWorkspaceRegistry(t *testing.T) {
	t.Parallel()

	r := NewInMemoryWorkspaceRegistry()
	if r == nil {
		t.Fatal("expected registry to be non-nil")
	}
	if r.workspaces == nil {
		t.Fatal("expected internal workspaces map to be initialized, got nil")
	}
}

func TestInMemoryWorkspaceRegistry_CommitAndGet(t *testing.T) {
	t.Parallel()

	t.Run("happy path: commit and retrieve existing record", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		id := values.NewWorkspaceId()

		manifest := domain.NewWorkspaceManifest(domain.ServiceSpec{
			Name:      "web",
			Command:   "go run main.go",
			Directory: "/app",
			Env:       map[string]string{"PORT": "8080"},
		})

		record, err := BuildWorkspaceRecord(manifest)
		if err != nil {
			t.Fatalf("failed to build record: %v", err)
		}

		r.Commit(id, record)

		got, ok := r.Get(id)
		if !ok {
			t.Fatalf("expected record for id %v to be found", id)
		}
		assertRecordEqual(t, record, got)
	})

	t.Run("fail path: get non-existent record", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		nonExistentID := values.NewWorkspaceId()

		got, ok := r.Get(nonExistentID)
		if ok {
			t.Fatalf("expected ok=false for non-existent workspace, got record %+v", got)
		}
		if len(got.ServiceHashes) != 0 || got.Hash != (hashing.Hash{}) {
			t.Fatalf("expected zero-value Record, got %+v", got)
		}
	})

	t.Run("edge case: overwrite existing record", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		id := values.NewWorkspaceId()

		initialManifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "v1"})
		initialRecord, _ := BuildWorkspaceRecord(initialManifest)

		updatedManifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "v2"})
		updatedRecord, _ := BuildWorkspaceRecord(updatedManifest)

		r.Commit(id, initialRecord)
		r.Commit(id, updatedRecord)

		got, ok := r.Get(id)
		if !ok {
			t.Fatalf("expected record for id %v to exist after overwrite", id)
		}
		assertRecordEqual(t, updatedRecord, got)
	})

	t.Run("edge case: zero value workspace ID", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		var zeroID values.WorkspaceId

		manifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "zero-svc"})
		record, _ := BuildWorkspaceRecord(manifest)

		r.Commit(zeroID, record)

		got, ok := r.Get(zeroID)
		if !ok {
			t.Fatalf("expected record with zero-value WorkspaceId to be stored and retrieved")
		}
		assertRecordEqual(t, record, got)
	})
}

func TestInMemoryWorkspaceRegistry_RaceAndConcurrency(t *testing.T) {
	t.Parallel()

	t.Run("concurrent commits and gets across multiple workspaces", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		const goroutines = 20
		const opsPerGoroutine = 100

		manifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "concurrent-svc"})
		record, _ := BuildWorkspaceRecord(manifest)

		var wg sync.WaitGroup
		wg.Add(goroutines * 2)

		for g := range goroutines {
			go func(gID int) {
				defer wg.Done()
				for range opsPerGoroutine {
					id := values.NewWorkspaceId()
					r.Commit(id, record)
				}
			}(g)
		}

		for range goroutines {
			go func() {
				defer wg.Done()
				for range opsPerGoroutine {
					id := values.NewWorkspaceId()
					_, _ = r.Get(id)
				}
			}()
		}

		wg.Wait()
	})

	t.Run("concurrent reads and writes on the exact same key", func(t *testing.T) {
		t.Parallel()

		r := NewInMemoryWorkspaceRegistry()
		targetID := values.NewWorkspaceId()

		manifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "target-svc"})
		record, _ := BuildWorkspaceRecord(manifest)

		const goroutines = 50
		const iterations = 200

		var wg sync.WaitGroup
		wg.Add(goroutines * 2)

		for g := range goroutines {
			go func(gID int) {
				defer wg.Done()
				for range iterations {
					r.Commit(targetID, record)
				}
			}(g)
		}

		for range goroutines {
			go func() {
				defer wg.Done()
				for range iterations {
					_, _ = r.Get(targetID)
				}
			}()
		}

		wg.Wait()

		if _, ok := r.Get(targetID); !ok {
			t.Fatal("expected target key to exist after concurrent execution")
		}
	})
}

func TestInMemoryWorkspaceRegistry_Isolation(t *testing.T) {
	t.Parallel()

	r := NewInMemoryWorkspaceRegistry()
	ids := make([]values.WorkspaceId, 10)

	manifest := domain.NewWorkspaceManifest(domain.ServiceSpec{Name: "iso-svc"})
	record, _ := BuildWorkspaceRecord(manifest)

	for i := range ids {
		ids[i] = values.NewWorkspaceId()
		r.Commit(ids[i], record)
	}

	for i, id := range ids {
		got, ok := r.Get(id)
		if !ok {
			t.Fatalf("expected element %d (id: %v) to exist", i, id)
		}
		assertRecordEqual(t, record, got)
	}
}
