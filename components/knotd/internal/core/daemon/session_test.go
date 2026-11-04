package daemon

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/requests"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/task"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/hashing"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/phase"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/workspace"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// failsafe turns a hang into a failure. Never used for synchronization.
const failsafe = 10 * time.Second

// ---------- fakes ----------

type journal struct {
	mu sync.Mutex
	e  []string
}

func (j *journal) add(s string) { j.mu.Lock(); j.e = append(j.e, s); j.mu.Unlock() }
func (j *journal) all() []string {
	j.mu.Lock()
	defer j.mu.Unlock()
	return append([]string(nil), j.e...)
}
func (j *journal) index(s string) int {
	for i, v := range j.all() {
		if v == s {
			return i
		}
	}
	return -1
}
func (j *journal) count(prefix string) int {
	n := 0
	for _, v := range j.all() {
		if len(v) >= len(prefix) && v[:len(prefix)] == prefix {
			n++
		}
	}
	return n
}

type fakeHandle struct {
	runtime.RunHandle
	name values.ServiceName
	j    *journal
}

func (h *fakeHandle) ID() string { return "pid-" + string(h.name) }
func (h *fakeHandle) Stop(context.Context, time.Duration) error {
	h.j.add("stop:" + string(h.name))
	return nil
}

type fakeWsRegistry struct {
	registry.WorkspaceRegistry
	mu   sync.Mutex
	recs map[values.WorkspaceID]workspace.Record
}

func (r *fakeWsRegistry) Get(id values.WorkspaceID) (workspace.Record, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	rec, ok := r.recs[id]
	return rec, ok
}

type fakeRtRegistry struct {
	registry.RuntimeRegistry
	mu    sync.Mutex
	items map[values.ServiceName]registry.ServiceHandle
}

func (r *fakeRtRegistry) Get(_ values.WorkspaceID, n values.ServiceName) (registry.ServiceHandle, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	h, ok := r.items[n]
	return h, ok
}
func (r *fakeRtRegistry) Register(h registry.ServiceHandle) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if _, dup := r.items[h.Service]; dup {
		return errors.New("already registered")
	}
	r.items[h.Service] = h
	return nil
}
func (r *fakeRtRegistry) Remove(sh registry.ServiceHandle) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.items, sh.Service)
	return nil
}
func (r *fakeRtRegistry) ListHashes(values.WorkspaceID) map[values.ServiceName]hashing.Hash {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make(map[values.ServiceName]hashing.Hash, len(r.items))
	for n, h := range r.items {
		out[n] = h.Hash
	}
	return out
}
func (r *fakeRtRegistry) has(n values.ServiceName) bool {
	_, ok := r.Get(values.NewWorkspaceId(), n)
	return ok
}

type fakeRuntime struct {
	runtime.Runtime
	j       *journal
	onStart func(ctx context.Context, name values.ServiceName) error // set before Up
}

func (r *fakeRuntime) Start(ctx context.Context, s domain.ServiceSpec) (runtime.RunHandle, error) {
	r.j.add("start:" + string(s.Name))
	if r.onStart != nil {
		if err := r.onStart(ctx, s.Name); err != nil {
			return nil, err
		}
	}
	return &fakeHandle{name: s.Name, j: r.j}, nil
}

// ---------- harness ----------

type env struct {
	t       *testing.T
	j       *journal
	lock    *core.WorkspaceLock
	wsReg   *fakeWsRegistry
	rtReg   *fakeRtRegistry
	rt      *fakeRuntime
	sess    *TaskSession
	sessCtx context.Context
}

func spec(name string, deps ...values.ServiceName) domain.ServiceSpec {
	return domain.ServiceSpec{Name: values.ServiceName(name), Command: "run-" + name, Depends: deps}
}

func newEnv(t *testing.T) *env {
	t.Helper()
	j := &journal{}
	e := &env{
		t: t, j: j, lock: core.NewWorkspaceLock(),
		wsReg: &fakeWsRegistry{recs: map[values.WorkspaceID]workspace.Record{}},
		rtReg: &fakeRtRegistry{items: map[values.ServiceName]registry.ServiceHandle{}},
		rt:    &fakeRuntime{j: j},
	}
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	e.sessCtx = ctx
	runner := phase.NewRunner(e.wsReg, e.rtReg, e.lock, e.rt)
	e.sess = NewSession(ctx, cancel, values.NewTaskID(), e.wsReg, e.rtReg, e.lock, runner)
	return e
}

// commit registers a manifest and returns its id and revision.
func (e *env) commit(specs ...domain.ServiceSpec) (values.WorkspaceID, string) {
	e.t.Helper()
	rec, err := workspace.BuildWorkspaceRecord(domain.NewWorkspaceManifest(specs...))
	if err != nil {
		e.t.Fatalf("BuildWorkspaceRecord: %v", err)
	}
	id := values.WorkspaceID(uuid.New())
	e.wsReg.mu.Lock()
	e.wsReg.recs[id] = rec
	e.wsReg.mu.Unlock()
	return id, rec.Hash.String()
}

// seed marks services as already running in the runtime registry.
func (e *env) seed(names ...values.ServiceName) {
	for i, n := range names {
		var h hashing.Hash
		h[0] = byte(100 + i)
		e.rtReg.items[n] = registry.ServiceHandle{Service: n, Hash: h, Handle: &fakeHandle{name: n, j: e.j}}
	}
}

func (e *env) up(ctx context.Context, id values.WorkspaceID, rev string, prune bool, svcs ...values.ServiceName) (<-chan task.TaskEvent, error) {
	return e.sess.Up(ctx, requests.UpRequest{WorkspaceId: id, ExpectedRevision: rev, Prune: prune, Services: svcs})
}

func (e *env) drain(ch <-chan task.TaskEvent) []task.TaskEvent {
	e.t.Helper()
	var evs []task.TaskEvent
	deadline := time.After(failsafe)
	for {
		select {
		case ev, ok := <-ch:
			if !ok {
				return evs
			}
			evs = append(evs, ev)
		case <-deadline:
			e.t.Fatalf("event channel not closed within %s (%d events)", failsafe, len(evs))
		}
	}
}

func (e *env) assertLockReleased(id values.WorkspaceID) {
	e.t.Helper()
	ok := make(chan struct{})
	go func() { e.lock.Lock(id); e.lock.Unlock(id); close(ok) }()
	select {
	case <-ok:
	case <-time.After(failsafe):
		e.t.Fatal("workspace lock was not released")
	}
}

func done(t *testing.T, evs []task.TaskEvent) task.TaskDone {
	t.Helper()
	if len(evs) == 0 {
		t.Fatal("no events")
	}
	d, ok := evs[len(evs)-1].(task.TaskDone)
	if !ok {
		t.Fatalf("last event is %T, want task.TaskDone", evs[len(evs)-1])
	}
	return d
}

func terminal(evs []task.TaskEvent, name string) task.Event {
	var out task.Event
	for _, ev := range evs {
		if se, ok := ev.(task.ServiceEvent); ok && se.Service == name {
			switch se.Event.(type) {
			case task.RunningEvent, task.StoppedEvent, task.SkippedEvent, task.FailedEvent:
				out = se.Event
			}
		}
	}
	return out
}

// ---------- happy paths ----------

func TestSession_Up_StartsAllInDependencyOrder(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("db"), spec("api", "db"), spec("web", "api"))

	ch, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	evs := e.drain(ch)

	if d := done(t, evs); d.Started != 3 || d.Failed != 0 {
		t.Fatalf("done = %+v, want Started=3", d)
	}
	if e.j.index("start:db") >= e.j.index("start:api") || e.j.index("start:api") >= e.j.index("start:web") {
		t.Fatalf("dependency order violated: %v", e.j.all())
	}
	acc, ok := evs[0].(task.TaskAccepted)
	if !ok || acc.TaskID != e.sess.id || len(acc.Services) != 3 {
		t.Fatalf("first event = %#v, want TaskAccepted with session task id and 3 services", evs[0])
	}
	e.assertLockReleased(id)
}

func TestSession_Up_ServiceSubsetStartsOnlyTargetAndItsDependencies(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("db"), spec("api", "db"), spec("metrics"))

	ch, err := e.up(context.Background(), id, rev, false, "api")
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	e.drain(ch)

	if e.rtReg.has("metrics") || !e.rtReg.has("api") || !e.rtReg.has("db") {
		t.Fatalf("unexpected runtime state; journal: %v", e.j.all())
	}
}

func TestSession_Up_PruneStopsOrphansBeforeStart(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("api"))
	e.seed("orphan-a", "orphan-b")

	ch, err := e.up(context.Background(), id, rev, true, "api")
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	evs := e.drain(ch)

	d := done(t, evs)
	if d.Stopped != 2 || d.Started != 1 {
		t.Fatalf("done = %+v, want Stopped=2 Started=1", d)
	}
	lastStop := max(e.j.index("stop:orphan-a"), e.j.index("stop:orphan-b"))
	if lastStop == -1 || lastStop > e.j.index("start:api") {
		t.Fatalf("prune must fully precede start: %v", e.j.all())
	}
	if e.rtReg.has("orphan-a") || e.rtReg.has("orphan-b") {
		t.Error("orphans must be removed from runtime registry")
	}
}

func TestSession_Down_StopsDependentsFirst(t *testing.T) {
	e := newEnv(t)
	id, _ := e.commit(spec("db"), spec("api", "db"))
	e.seed("db", "api")

	ch, err := e.sess.Down(context.Background(), requests.DownRequest{WorkspaceID: id})
	if err != nil {
		t.Fatalf("Down: %v", err)
	}
	evs := e.drain(ch)

	if d := done(t, evs); d.Stopped != 2 {
		t.Fatalf("done = %+v, want Stopped=2", d)
	}
	if e.j.index("stop:api") > e.j.index("stop:db") {
		t.Fatalf("dependent must stop first: %v", e.j.all())
	}
	e.assertLockReleased(id)
}

func TestSession_Down_ServiceStopsItsDependentsButNotItsDependencies(t *testing.T) {
	e := newEnv(t)
	id, _ := e.commit(spec("db"), spec("api", "db"), spec("web", "api"))
	e.seed("db", "api", "web")

	ch, err := e.sess.Down(context.Background(), requests.DownRequest{WorkspaceID: id, Services: []values.ServiceName{"api"}})
	if err != nil {
		t.Fatalf("Down: %v", err)
	}
	e.drain(ch)

	if !e.rtReg.has("db") || e.rtReg.has("api") || e.rtReg.has("web") {
		t.Fatalf("down -s api must stop api+web and keep db; journal: %v", e.j.all())
	}
}

// ---------- fail paths ----------

func TestSession_Up_ErrorPaths(t *testing.T) {
	t.Run("workspace does not exist", func(t *testing.T) {
		e := newEnv(t)
		ghost := values.WorkspaceID(uuid.New())

		ch, err := e.up(context.Background(), ghost, "rev", false)

		var nf ErrWorkspaceNotExists
		if !errors.As(err, &nf) {
			t.Fatalf("err = %v, want ErrWorkspaceNotExists", err)
		}
		if ch != nil {
			t.Error("channel must be nil on error")
		}
		e.assertLockReleased(ghost)
	})

	t.Run("wrong revision", func(t *testing.T) {
		e := newEnv(t)
		id, _ := e.commit(spec("api"))

		ch, err := e.up(context.Background(), id, "stale-revision", false)

		var wr ErrWrongRevision
		if !errors.As(err, &wr) {
			t.Fatalf("err = %v, want ErrWrongRevision", err)
		}
		if ch != nil || e.j.count("start:") != 0 {
			t.Error("nothing may run on revision mismatch")
		}
		e.assertLockReleased(id)
	})

	t.Run("unknown requested service", func(t *testing.T) {
		e := newEnv(t)
		id, rev := e.commit(spec("api"))

		ch, err := e.up(context.Background(), id, rev, true, "missing")

		if err == nil || ch != nil {
			t.Fatalf("want error and nil channel, got err=%v ch=%v", err, ch)
		}
		if e.j.count("start:") != 0 || e.j.count("stop:") != 0 {
			t.Error("validation errors must surface before any execution (including prune)")
		}
		e.assertLockReleased(id)
	})

	t.Run("dependency cycle in manifest", func(t *testing.T) {
		e := newEnv(t)
		id, rev := e.commit(spec("a", "b"), spec("b", "a"))

		ch, err := e.up(context.Background(), id, rev, false)

		if err == nil || ch != nil {
			t.Fatalf("want error and nil channel, got err=%v ch=%v", err, ch)
		}
		e.assertLockReleased(id)
	})
}

func TestSession_Down_UnknownServiceFailsSynchronously(t *testing.T) {
	e := newEnv(t)
	id, _ := e.commit(spec("api"))
	e.seed("api")

	ch, err := e.sess.Down(context.Background(), requests.DownRequest{WorkspaceID: id, Services: []values.ServiceName{"missing"}})

	if err == nil || ch != nil {
		t.Fatalf("want error and nil channel, got err=%v ch=%v", err, ch)
	}
	if e.j.count("stop:") != 0 || !e.rtReg.has("api") {
		t.Error("nothing may be stopped on validation error")
	}
	e.assertLockReleased(id)
}

func TestSession_Up_StartFailureSkipsDependentsAndStillCompletes(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("db"), spec("api", "db"), spec("metrics"))
	e.rt.onStart = func(_ context.Context, n values.ServiceName) error {
		if n == "db" {
			return errors.New("exec: not found")
		}
		return nil
	}

	ch, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	evs := e.drain(ch)

	if _, ok := terminal(evs, "db").(task.FailedEvent); !ok {
		t.Errorf("db terminal = %T, want FailedEvent", terminal(evs, "db"))
	}
	if _, ok := terminal(evs, "api").(task.SkippedEvent); !ok {
		t.Errorf("api terminal = %T, want SkippedEvent (dependency failed)", terminal(evs, "api"))
	}
	if _, ok := terminal(evs, "metrics").(task.RunningEvent); !ok {
		t.Errorf("independent service must still start, got %T", terminal(evs, "metrics"))
	}
	if d := done(t, evs); d.Failed != 1 || d.Skipped != 1 || d.Started != 1 {
		t.Fatalf("done = %+v, want Failed=1 Skipped=1 Started=1", d)
	}
	e.assertLockReleased(id)
}

// ---------- edge cases ----------

func TestSession_Up_PruneDisabledLeavesOrphansUntouched(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("api"))
	e.seed("orphan")

	ch, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	e.drain(ch)

	if !e.rtReg.has("orphan") || e.j.count("stop:") != 0 {
		t.Fatalf("orphan must survive without --prune: %v", e.j.all())
	}
}

func TestSession_Up_PruneWithoutOrphansIsNoOp(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("api"))

	ch, err := e.up(context.Background(), id, rev, true)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	if d := done(t, e.drain(ch)); d.Stopped != 0 || d.Started != 1 {
		t.Fatalf("done = %+v", d)
	}
}

func TestSession_Up_SecondRunIsIdempotent(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("api"))

	for i := 0; i < 2; i++ {
		ch, err := e.up(context.Background(), id, rev, false)
		if err != nil {
			t.Fatalf("Up #%d: %v", i, err)
		}
		evs := e.drain(ch)
		d := done(t, evs)
		if i == 0 && d.Started != 1 || i == 1 && (d.Started != 0 || d.Skipped != 1) {
			t.Fatalf("run %d: done = %+v", i, d)
		}
	}
	if e.j.count("start:api") != 1 {
		t.Fatalf("runtime.Start must be called once, journal: %v", e.j.all())
	}
}

func TestSession_Up_EmptyManifestCompletesImmediately(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit()

	ch, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	if d := done(t, e.drain(ch)); d != (task.TaskDone{TaskID: e.sess.id}) {
		t.Fatalf("done = %+v, want zero stats", d)
	}
	e.assertLockReleased(id)
}

// ---------- cancellation & lifecycle ----------

// blockInStart makes Start signal entry and then wait for ctx cancellation.
func blockInStart(entered chan<- struct{}) func(context.Context, values.ServiceName) error {
	var once sync.Once
	return func(ctx context.Context, _ values.ServiceName) error {
		once.Do(func() { close(entered) })
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(failsafe):
			return errors.New("not cancelled")
		}
	}
}

func TestSession_Cancel_StopsRunAndReleasesLock(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("slow"))
	entered := make(chan struct{})
	e.rt.onStart = blockInStart(entered)

	ch, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	<-entered
	e.sess.Cancel()

	evs := e.drain(ch)
	if d := done(t, evs); d.Started != 0 {
		t.Fatalf("done = %+v, cancelled service must not count as started", d)
	}
	e.assertLockReleased(id)
}

func TestSession_CallerContextCancel_StopsRunAndReleasesLock(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("slow"))
	entered := make(chan struct{})
	e.rt.onStart = blockInStart(entered)
	callerCtx, cancelCaller := context.WithCancel(context.Background())
	t.Cleanup(cancelCaller)

	ch, err := e.up(callerCtx, id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	<-entered
	cancelCaller()

	// Consumer is gone: events may be dropped, but the channel must close.
	e.drain(ch)
	e.assertLockReleased(id)
}

func TestSession_ConsumerNeverReads_DoesNotLeakLock(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("a"), spec("b", "a"), spec("c", "b"))
	callerCtx, cancelCaller := context.WithCancel(context.Background())
	t.Cleanup(cancelCaller)

	ch, err := e.up(callerCtx, id, rev, false)
	if err != nil {
		t.Fatalf("Up: %v", err)
	}
	_ = ch
	cancelCaller() // consumer disappears without ever reading

	e.assertLockReleased(id) // would hang if Execute blocks on emit
}

// ---------- concurrency ----------

// Two Up on one workspace must be serialized by the workspace lock. Proof is
// behavioural: the second run observes the first run's result (Skipped), which
// is impossible if both executed concurrently.
func TestSession_ConcurrentUpOnSameWorkspace_AreSerialized(t *testing.T) {
	e := newEnv(t)
	id, rev := e.commit(spec("api"))

	entered := make(chan struct{})
	gate := make(chan struct{})
	var once sync.Once
	e.rt.onStart = func(ctx context.Context, _ values.ServiceName) error {
		once.Do(func() { close(entered) })
		select {
		case <-gate:
			return nil
		case <-time.After(failsafe):
			return errors.New("gate never opened")
		}
	}

	first, err := e.up(context.Background(), id, rev, false)
	if err != nil {
		t.Fatalf("first Up: %v", err)
	}
	<-entered // first run holds the lock and is inside Start

	type result struct {
		ch  <-chan task.TaskEvent
		err error
	}
	second := make(chan result, 1)
	go func() {
		ch, err := e.up(context.Background(), id, rev, false)
		second <- result{ch, err}
	}()

	close(gate)
	firstEvs := e.drain(first)

	var r result
	select {
	case r = <-second:
	case <-time.After(failsafe):
		t.Fatal("second Up never acquired the lock")
	}
	if r.err != nil {
		t.Fatalf("second Up: %v", r.err)
	}
	secondEvs := e.drain(r.ch)

	if d := done(t, firstEvs); d.Started != 1 {
		t.Fatalf("first done = %+v", d)
	}
	if d := done(t, secondEvs); d.Started != 0 || d.Skipped != 1 {
		t.Fatalf("second done = %+v, want Skipped=1 (serialized after first)", d)
	}
	if e.j.count("start:api") != 1 {
		t.Fatalf("service started %d times, want 1", e.j.count("start:api"))
	}
	e.assertLockReleased(id)
}

// Different workspaces must not block each other. Rendezvous: each Start waits
// until BOTH have entered; a global lock would make this time out and fail.
func TestSession_UpOnDifferentWorkspaces_RunInParallel(t *testing.T) {
	e := newEnv(t)
	id1, rev1 := e.commit(spec("one"))
	id2, rev2 := e.commit(spec("two"))

	var entered sync.WaitGroup
	entered.Add(2)
	release := make(chan struct{})
	go func() { entered.Wait(); close(release) }()
	e.rt.onStart = func(context.Context, values.ServiceName) error {
		entered.Done()
		select {
		case <-release:
			return nil
		case <-time.After(3 * time.Second):
			return errors.New("workspaces are serialized")
		}
	}

	ch1, err := e.up(context.Background(), id1, rev1, false)
	if err != nil {
		t.Fatalf("Up ws1: %v", err)
	}
	ch2, err := e.up(context.Background(), id2, rev2, false)
	if err != nil {
		t.Fatalf("Up ws2: %v", err)
	}

	for i, ch := range []<-chan task.TaskEvent{ch1, ch2} {
		if d := done(t, e.drain(ch)); d.Started != 1 || d.Failed != 0 {
			t.Fatalf("ws%d done = %+v, want Started=1", i+1, d)
		}
	}
}

func TestSession_Up_DoesNotReturnBeforePlanIsBuilt_NoPartialEventsOnError(t *testing.T) {
	// A validation error must never be preceded by a TaskAccepted event:
	// the plan is built synchronously under the lock, before the goroutine exists.
	e := newEnv(t)
	id, rev := e.commit(spec("api"))

	ch, err := e.up(context.Background(), id, rev, false, "nope")

	if err == nil {
		t.Fatal("expected error")
	}
	if ch != nil {
		t.Fatal("no channel (hence no TaskAccepted) may exist on validation error")
	}
}
