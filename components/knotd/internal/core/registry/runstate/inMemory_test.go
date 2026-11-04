package runstate

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"sync"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// mockRunHandle implements runtime.RunHandle for testing purposes.
type mockRunHandle struct {
	id         string
	status     runtime.Status
	stopErr    error
	inspectErr error
}

func (m mockRunHandle) ID() string {
	return m.id
}

func (m mockRunHandle) Stop(context.Context, time.Duration) error {
	return m.stopErr
}

func (m mockRunHandle) Inspect(context.Context) (runtime.Status, error) {
	if m.inspectErr != nil {
		return runtime.Status{}, m.inspectErr
	}
	return m.status, nil
}

func makeHandle(id values.WorkspaceID, name string, h *mockRunHandle) registry.ServiceHandle {
	return registry.ServiceHandle{
		Workspace: id,
		Service:   values.ServiceName(name),
		Handle:    h,
		Hash:      hashing.Hash{},
	}
}

func assertHandleEqual(t *testing.T, expected, got registry.ServiceHandle) {
	t.Helper()
	if got.Workspace != expected.Workspace || got.Service != expected.Service || got.Hash != expected.Hash {
		t.Fatalf("handle mismatch: expected %+v, got %+v", expected, got)
	}
	if got.Handle != expected.Handle {
		if got.Handle == nil || expected.Handle == nil || got.Handle.ID() != expected.Handle.ID() {
			t.Fatalf("handle RunHandle mismatch: expected %v, got %v", expected.Handle, got.Handle)
		}
	}
}

func TestInMemoryRuntimeRegistry_Register(t *testing.T) {
	t.Run("successful registration", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		serviceHandle := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})

		err := r.Register(serviceHandle)
		if err != nil {
			t.Fatalf("expected no error, got: %v", err)
		}

		gotHandle, ok := r.Get(serviceHandle.Workspace, "svc1")
		if !ok {
			t.Fatal("expected service to be registered and found")
		}
		assertHandleEqual(t, serviceHandle, gotHandle)
	})

	t.Run("fail path: duplicate registration returns ErrAlreadyRegistered", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		serviceHandle1 := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		serviceHandle2 := makeHandle(id, "svc1", &mockRunHandle{id: "h2"})

		err := r.Register(serviceHandle1)
		if err != nil {
			t.Fatalf("unexpected error on first registration: %v", err)
		}

		err = r.Register(serviceHandle2)
		if err == nil {
			t.Fatal("expected error on duplicate registration, got nil")
		}
		if !errors.Is(err, registry.ErrAlreadyRegistered) {
			t.Errorf("expected error to wrap ErrAlreadyRegistered, got: %v", err)
		}

		// Ensure original handle was not overwritten
		gotHandle, ok := r.Get(id, "svc1")
		if !ok {
			t.Errorf("handle was overwritten on failed registration attempt")
		}

		assertHandleEqual(t, serviceHandle1, gotHandle)
	})

	t.Run("same service name in different workspaces", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id1 := values.NewWorkspaceId()
		id2 := values.NewWorkspaceId()
		handle1 := makeHandle(id1, "svc1", &mockRunHandle{id: "h1"})
		handle2 := makeHandle(id2, "svc1", &mockRunHandle{id: "h2"})

		if err := r.Register(handle1); err != nil {
			t.Fatalf("failed registering ws1/svc1: %v", err)
		}
		if err := r.Register(handle2); err != nil {
			t.Fatalf("failed registering ws2/svc1: %v", err)
		}

		h1, ok1 := r.Get(id1, "svc1")
		h2, ok2 := r.Get(id2, "svc1")

		if !ok1 {
			t.Errorf("ws1/svc1 corrupted")
		}
		assertHandleEqual(t, handle1, h1)
		if !ok2 {
			t.Errorf("ws2/svc1 corrupted")
		}
		assertHandleEqual(t, handle2, h2)
	})
}

func TestInMemoryRuntimeRegistry_Get(t *testing.T) {
	r := NewInMemoryRuntimeRegistry()
	id := values.NewWorkspaceId()
	serviceHandle := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
	_ = r.Register(serviceHandle)

	t.Run("get existing service", func(t *testing.T) {
		h, ok := r.Get(id, "svc1")
		if !ok || h != serviceHandle {
			t.Errorf("expected handle %v, got %v (ok=%v)", serviceHandle, h, ok)
		}
	})

	t.Run("get non-existent workspace", func(t *testing.T) {
		h, ok := r.Get(values.NewWorkspaceId(), "svc1")
		if ok {
			t.Errorf("expected nil, false for non-existent workspace, got %v, %v", h, ok)
		}
	})

	t.Run("get non-existent service in existing workspace", func(t *testing.T) {
		h, ok := r.Get(id, "non-existent-svc")
		if ok {
			t.Errorf("expected nil, false for non-existent service, got %v, %v", h, ok)
		}
	})
}

func TestInMemoryRuntimeRegistry_Remove(t *testing.T) {
	prune := func(t *testing.T, r *InMemoryRuntimeRegistry, id values.WorkspaceID) bool {
		t.Helper()
		r.mu.RLock()
		defer r.mu.RUnlock()
		_, exists := r.byWorkspace[id]
		return exists
	}

	t.Run("matching handle is removed, siblings untouched", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		h1 := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		h2 := makeHandle(id, "svc2", &mockRunHandle{id: "h2"})
		_ = r.Register(h1)
		_ = r.Register(h2)

		if err := r.Remove(h1); err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if _, ok := r.Get(id, "svc1"); ok {
			t.Error("svc1 should have been removed")
		}
		if _, ok := r.Get(id, "svc2"); !ok {
			t.Error("svc2 should still exist")
		}
	})

	t.Run("workspace cleanup after last service removed", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		h := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		_ = r.Register(h)

		if err := r.Remove(h); err != nil {
			t.Fatal(err)
		}
		if list := r.List(id); list != nil {
			t.Errorf("expected List to return nil after cleanup, got %+v", list)
		}
		if prune(t, r, id) {
			t.Error("expected workspace entry to be pruned from internal map")
		}
	})

	t.Run("different handle with same ID() is NOT removed (identity, not ID)", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		live := &mockRunHandle{id: "h1"}
		_ = r.Register(makeHandle(id, "svc1", live))

		// same ID on purpose: models PID reuse
		err := r.Remove(makeHandle(id, "svc1", &mockRunHandle{id: "h1"}))
		if !errors.Is(err, registry.ErrHandleNotMatch) {
			t.Fatalf("want ErrHandleNotMatch, got %v", err)
		}
		cur, ok := r.Get(id, "svc1")
		if !ok {
			t.Fatal("live entry was evicted by a stale handle")
		}
		if cur.Handle != live {
			t.Error("registry no longer holds the live handle")
		}
	})

	t.Run("replacement scenario: old handle cannot evict new instance", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		old := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		fresh := &mockRunHandle{id: "h1"}
		_ = r.Register(old)

		if err := r.Remove(old); err != nil { // regular Down
			t.Fatal(err)
		}
		if err := r.Register(makeHandle(id, "svc1", fresh)); err != nil { // Up
			t.Fatal(err)
		}

		err := r.Remove(old) // stale supervisor event
		if !errors.Is(err, registry.ErrHandleNotMatch) {
			t.Fatalf("want ErrHandleNotMatch, got %v", err)
		}
		if cur, ok := r.Get(id, "svc1"); !ok || cur.Handle != fresh {
			t.Error("replacement instance must remain registered")
		}
	})

	t.Run("mismatch does not prune workspace", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		_ = r.Register(makeHandle(id, "svc1", &mockRunHandle{id: "h1"}))

		err := r.Remove(makeHandle(id, "svc1", &mockRunHandle{id: "h1"}))
		if !errors.Is(err, registry.ErrHandleNotMatch) {
			t.Fatalf("want ErrHandleNotMatch, got %v", err)
		}
		if !prune(t, r, id) {
			t.Error("workspace entry pruned despite handle mismatch")
		}
	})

	t.Run("no-op: missing service, missing workspace, repeated call", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		h := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		_ = r.Register(h)

		if err := r.Remove(makeHandle(id, "non-existent", &mockRunHandle{id: "h1"})); err != nil {
			t.Errorf("missing service: want nil, got %v", err)
		}
		if err := r.Remove(makeHandle(values.NewWorkspaceId(), "svc1", &mockRunHandle{id: "h1"})); err != nil {
			t.Errorf("missing workspace: want nil, got %v", err)
		}
		if _, ok := r.Get(id, "svc1"); !ok {
			t.Error("svc1 was removed by an invalid operation")
		}

		// Idempotency: removal, then a repeat on the now-absent entry.
		if err := r.Remove(h); err != nil {
			t.Fatalf("first call: %v", err)
		}
		if _, ok := r.Get(id, "svc1"); ok {
			t.Fatal("first call must remove")
		}
		if err := r.Remove(h); err != nil {
			t.Errorf("second call: want nil, got %v", err)
		}
	})

	t.Run("nil handle never matches", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		_ = r.Register(makeHandle(id, "svc1", &mockRunHandle{id: "h1"}))

		err := r.Remove(makeHandle(id, "svc1", nil))
		if !errors.Is(err, registry.ErrHandleNotMatch) {
			t.Errorf("want ErrHandleNotMatch, got %v", err)
		}
		if _, ok := r.Get(id, "svc1"); !ok {
			t.Error("nil handle evicted a live entry")
		}
	})

	t.Run("concurrent: all callers see nil, entry removed, no races", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		h := makeHandle(id, "svc1", &mockRunHandle{id: "h1"})
		_ = r.Register(h)

		const n = 32
		var wg sync.WaitGroup
		start := make(chan struct{})
		for range n {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				if err := r.Remove(h); err != nil {
					t.Errorf("unexpected error: %v", err)
				}
			}()
		}
		close(start)
		wg.Wait()

		if _, ok := r.Get(id, "svc1"); ok {
			t.Error("entry must be gone")
		}
	})

	t.Run("concurrent: stale handle never evicts live entry", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		id := values.NewWorkspaceId()
		live := &mockRunHandle{id: "h1"}
		_ = r.Register(makeHandle(id, "svc1", live))

		const n = 32
		var wg sync.WaitGroup
		start := make(chan struct{})
		for range n {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				err := r.Remove(makeHandle(id, "svc1", &mockRunHandle{id: "h1"}))
				if !errors.Is(err, registry.ErrHandleNotMatch) {
					t.Errorf("want ErrHandleNotMatch, got %v", err)
				}
			}()
		}
		close(start)
		wg.Wait()

		if cur, ok := r.Get(id, "svc1"); !ok || cur.Handle != live {
			t.Error("live entry must survive concurrent stale removals")
		}
	})
}

func TestInMemoryRuntimeRegistry_ListAndListAll(t *testing.T) {
	r := NewInMemoryRuntimeRegistry()
	id1 := values.NewWorkspaceId()
	id2 := values.NewWorkspaceId()

	t.Run("empty registry listing", func(t *testing.T) {
		if res := r.List(id1); res != nil {
			t.Errorf("expected nil for empty workspace List, got %+v", res)
		}
		if res := r.ListAll(); len(res) != 0 {
			t.Errorf("expected empty slice for empty ListAll, got %+v", res)
		}
	})

	h1 := makeHandle(id1, "svc1", &mockRunHandle{id: "1"})
	h2 := makeHandle(id1, "svc2", &mockRunHandle{id: "2"})
	h3 := makeHandle(id2, "svc3", &mockRunHandle{id: "3"})

	_ = r.Register(h1)
	_ = r.Register(h2)
	_ = r.Register(h3)

	t.Run("List specific workspace", func(t *testing.T) {
		list := r.List(id1)
		if len(list) != 2 {
			t.Fatalf("expected 2 handles in ws1, got %d", len(list))
		}

		names := []string{string(list[0].Service), string(list[1].Service)}
		slices.Sort(names)
		if names[0] != "svc1" || names[1] != "svc2" {
			t.Errorf("unexpected services in ws1: %v", names)
		}
	})

	t.Run("ListAll across all workspaces", func(t *testing.T) {
		all := r.ListAll()
		if len(all) != 3 {
			t.Fatalf("expected 3 handles total, got %d", len(all))
		}
	})

	t.Run("ListHashes specific workspace", func(t *testing.T) {
		list := r.ListHashes(id1)
		if len(list) != 2 {
			t.Fatalf("expected 2 entries in ws1, got %d", len(list))
		}

		if h1.Hash != list[h1.Service] {
			t.Fatalf("expected %q hash in ws1, got %q", h1.Hash, list[h1.Service])
		}
	})
}

func TestInMemoryRuntimeRegistry_Concurrency(t *testing.T) {
	t.Run("concurrent reads and writes across multiple workspaces", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		const goroutines = 20
		const opsPerGoroutine = 100

		var wg sync.WaitGroup
		wg.Add(goroutines * 3)

		// Concurrent Writers (Register)
		for g := range goroutines {
			go func(gID int) {
				defer wg.Done()
				for i := range opsPerGoroutine {
					ws := values.NewWorkspaceId()
					svc := fmt.Sprintf("svc-%d-%d", gID, i)
					_ = r.Register(makeHandle(ws, svc, &mockRunHandle{id: svc}))
				}
			}(g)
		}

		// Concurrent Readers (Get & List)
		for range goroutines {
			go func() {
				defer wg.Done()
				for range opsPerGoroutine {
					ws := values.NewWorkspaceId()
					_, _ = r.Get(ws, "svc-0-0")
					_ = r.List(ws)
				}
			}()
		}

		// Concurrent Removers
		for g := range goroutines {
			go func(gID int) {
				defer wg.Done()
				for i := range opsPerGoroutine {
					ws := values.NewWorkspaceId()
					svc := fmt.Sprintf("svc-%d-%d", gID, i)
					err := r.Remove(makeHandle(ws, svc, &mockRunHandle{id: svc}))
					if err != nil {
						t.Error(err)
					}
				}
			}(g)
		}

		wg.Wait()
	})

	t.Run("concurrent ListAll while mutating state", func(t *testing.T) {
		r := NewInMemoryRuntimeRegistry()
		var wg sync.WaitGroup
		stop := make(chan struct{})

		// Mutator goroutines
		wg.Add(2)
		go func() {
			defer wg.Done()
			i := 0
			for {
				select {
				case <-stop:
					return
				default:
					ws := values.NewWorkspaceId()
					svcName := fmt.Sprintf("svc-%d", i)
					sh := makeHandle(ws, svcName, &mockRunHandle{id: "h"})
					_ = r.Register(sh)
					err := r.Remove(sh)
					if err != nil {
						t.Error(err)
					}
					i++
				}
			}
		}()

		go func() {
			defer wg.Done()
			for {
				select {
				case <-stop:
					return
				default:
					_ = r.ListAll()
				}
			}
		}()

		// Run race test for a short cycle
		for range 50 {
			_ = r.ListAll()
		}

		close(stop)
		wg.Wait()
	})
}
