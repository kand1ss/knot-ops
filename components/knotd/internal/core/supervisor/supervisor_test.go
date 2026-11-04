package supervisor_test

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry/runstate"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

const (
	tickInterval = 2 * time.Millisecond
	waitFor      = 3 * time.Second
	quietWindow  = 60 * time.Millisecond // negative assertions only: ~30 ticks
)

// ---------- fakes ----------

// fakeHandle is a pointer type: identity == pointer equality.
type fakeHandle struct {
	alive    atomic.Bool
	code     atomic.Int32
	inspects atomic.Int64
	inspErr  atomic.Pointer[error]
}

func newFake(alive bool) *fakeHandle {
	f := &fakeHandle{}
	f.alive.Store(alive)
	return f
}

func (f *fakeHandle) kill(code int32)       { f.code.Store(code); f.alive.Store(false) }
func (f *fakeHandle) failInspect(err error) { f.inspErr.Store(&err) }

func (f *fakeHandle) ID() string                                { return "fake" } // same ID on purpose: identity != ID()
func (f *fakeHandle) Stop(context.Context, time.Duration) error { return nil }
func (f *fakeHandle) Inspect(context.Context) (runtime.Status, error) {
	f.inspects.Add(1)
	if p := f.inspErr.Load(); p != nil {
		return runtime.Status{}, *p
	}
	alive := f.alive.Load()
	st := runtime.Status{Alive: alive}
	if !alive {
		c := f.code.Load()
		st.ExitCode = &c
	}
	return st, nil
}

// ---------- harness ----------

type rig struct {
	t   *testing.T
	reg *runstate.InMemoryRuntimeRegistry
	sup *supervisor.Supervisor
	ws  values.WorkspaceID
}

func newRig(t *testing.T) *rig {
	t.Helper()
	reg := runstate.NewInMemoryRuntimeRegistry()
	sup, err := supervisor.NewSupervisor(reg, tickInterval)
	if err != nil {
		t.Fatalf("NewSupervisor: %v", err)
	}
	return &rig{t: t, reg: reg, sup: sup, ws: values.NewWorkspaceId()}
}

// run starts the supervisor and verifies on cleanup that Run returns.
func (r *rig) run() context.CancelFunc {
	r.t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { defer close(done); r.sup.Run(ctx) }()
	r.t.Cleanup(func() {
		cancel()
		select {
		case <-done:
		case <-time.After(waitFor):
			r.t.Error("Run did not return after cancel")
		}
	})
	return cancel
}

func (r *rig) add(name string, h runtime.RunHandle) registry.ServiceHandle {
	r.t.Helper()
	sh := registry.ServiceHandle{Workspace: r.ws, Service: values.ServiceName(name), Handle: h}
	if err := r.reg.Register(sh); err != nil {
		r.t.Fatalf("register %s: %v", name, err)
	}
	return sh
}

func (r *rig) recv() (supervisor.Event, bool) {
	select {
	case ev, ok := <-r.sup.Events():
		return ev, ok
	case <-time.After(waitFor):
		r.t.Fatal("timeout waiting for event")
		return supervisor.Event{}, false
	}
}

// expectQuiet fails if any event arrives within the quiet window.
func (r *rig) expectQuiet(what string) {
	r.t.Helper()
	select {
	case ev := <-r.sup.Events():
		r.t.Fatalf("%s: unexpected event %+v", what, ev)
	case <-time.After(quietWindow):
	}
}

func eventually(t *testing.T, cond func() bool, what string) {
	t.Helper()
	deadline := time.Now().Add(waitFor)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatalf("condition not reached: %s", what)
}

// ---------- construction ----------

func TestNewSupervisor_RejectsNonPositiveInterval(t *testing.T) {
	reg := runstate.NewInMemoryRuntimeRegistry()
	for _, d := range []time.Duration{0, -time.Millisecond} {
		if _, err := supervisor.NewSupervisor(reg, d); err == nil {
			t.Errorf("interval %v accepted", d)
		}
	}
}

func TestNewSupervisor_RejectsNilRegistry(t *testing.T) {
	if _, err := supervisor.NewSupervisor(nil, tickInterval); err == nil {
		t.Error("nil registry accepted")
	}
}

// ---------- detection ----------

func TestSupervisor_EventCarriesHandleStatusAndExitCode(t *testing.T) {
	r := newRig(t)
	h := newFake(true)
	sh := r.add("svc", h)
	r.run()

	h.kill(7)
	ev, ok := r.recv()
	if !ok {
		t.Fatal("channel closed")
	}
	if ev.Status != supervisor.EventExited {
		t.Errorf("status=%v", ev.Status)
	}
	if ev.Handle.Service != sh.Service || ev.Handle.Workspace != sh.Workspace {
		t.Errorf("wrong identity: %+v", ev.Handle)
	}
	if ev.Handle.Handle != runtimeHandle(h) {
		t.Error("event must carry the exact handle instance (needed for compare-and-remove)")
	}
	if ev.ExitCode == nil || *ev.ExitCode != 7 {
		t.Errorf("exit code=%v, want 7", ev.ExitCode)
	}
}

func runtimeHandle(h *fakeHandle) runtime.RunHandle { return h }

func TestSupervisor_AliveServiceProducesNoEvents(t *testing.T) {
	r := newRig(t)
	h := newFake(true)
	r.add("svc", h)
	r.run()

	r.expectQuiet("alive service")
	if h.inspects.Load() < 5 {
		t.Errorf("service was inspected only %d times; ticker not running?", h.inspects.Load())
	}
}

func TestSupervisor_EmptyRegistryIsHarmless(t *testing.T) {
	r := newRig(t)
	r.run()
	r.expectQuiet("empty registry")
}

// ---------- deduplication (RED until `reported` is implemented) ----------

// An exited service that the consumer has not removed must yield exactly ONE
// event, not one per tick.
func TestSupervisor_ExitReportedExactlyOnce(t *testing.T) {
	r := newRig(t)
	h := newFake(false)
	r.add("svc", h)
	r.run()

	if _, ok := r.recv(); !ok {
		t.Fatal("channel closed")
	}
	// Entry intentionally NOT removed: simulates a slow consumer.
	r.expectQuiet("duplicate exit event for an already-reported service")
}

func TestSupervisor_DedupIsPerService(t *testing.T) {
	r := newRig(t)
	a, b := newFake(false), newFake(false)
	r.add("a", a)
	r.add("b", b)
	r.run()

	seen := map[values.ServiceName]int{}
	for range 2 {
		ev, _ := r.recv()
		seen[ev.Handle.Service]++
	}
	if seen["a"] != 1 || seen["b"] != 1 {
		t.Fatalf("want one event per service, got %v", seen)
	}
	r.expectQuiet("after both reported")
}

// Same name, NEW handle (restart) must be reported again.
func TestSupervisor_RestartUnderSameNameIsReportedAgain(t *testing.T) {
	r := newRig(t)
	old := newFake(false)
	oldSH := r.add("svc", old)
	r.run()

	first, _ := r.recv()
	if first.Handle.Handle != oldSH.Handle {
		t.Fatal("first event for wrong instance")
	}

	// Consumer removes the dead instance; Up registers a replacement that dies too.
	if err := r.reg.Remove(oldSH); err != nil {
		t.Fatal(err)
	}
	fresh := newFake(false)
	freshSH := r.add("svc", fresh)

	second, _ := r.recv()
	if second.Handle.Handle != freshSH.Handle {
		t.Fatal("restarted instance was not reported (dedup key ignores instance identity)")
	}
}

// Replacement happening between two ticks, without the old key ever being
// observed as removed: still a different handle, still reported.
func TestSupervisor_ReplacementWithoutIntermediateTickIsReported(t *testing.T) {
	r := newRig(t)
	old := newFake(false)
	oldSH := r.add("svc", old)
	r.run()
	_, _ = r.recv()

	_ = r.reg.Remove(oldSH)
	fresh := newFake(true) // alive: must NOT be reported
	r.add("svc", fresh)
	r.expectQuiet("alive replacement")

	fresh.kill(3)
	ev, _ := r.recv()
	if ev.ExitCode == nil || *ev.ExitCode != 3 {
		t.Errorf("exit code=%v, want 3", ev.ExitCode)
	}
}

// GC: after the entry is removed, supervisor state must not grow forever.
// Observable proxy: many register/exit/remove cycles keep working.
func TestSupervisor_ManyLifecyclesKeepReporting(t *testing.T) {
	r := newRig(t)
	r.run()

	for i := range 200 {
		h := newFake(false)
		sh := r.add("svc", h)
		ev, _ := r.recv()
		if ev.Handle.Handle != sh.Handle {
			t.Fatalf("cycle %d: wrong instance reported", i)
		}
		if err := r.reg.Remove(sh); err != nil {
			t.Fatalf("cycle %d: %v", i, err)
		}
	}
}

// ---------- back-pressure (RED until non-blocking delivery) ----------

// A stuck consumer must not stop inspection of other services.
func TestSupervisor_StuckConsumerDoesNotBlockInspection(t *testing.T) {
	r := newRig(t)

	// Enough dead services to exceed any reasonable buffer; nobody reads Events().
	const dead = 512
	for i := range dead {
		r.add("dead-"+itoa(i), newFake(false))
	}
	alive := newFake(true)
	r.add("alive", alive)
	r.run()

	// Let the channel fill up, then take a baseline.
	time.Sleep(quietWindow)
	before := alive.inspects.Load()
	time.Sleep(quietWindow)
	after := alive.inspects.Load()

	if after-before < 5 {
		t.Fatalf("inspection stalled while consumer is stuck: %d inspects in %v", after-before, quietWindow)
	}
}

// Nothing may be lost: once the consumer wakes up it must receive an event
// for every dead service exactly once.
func TestSupervisor_SlowConsumerEventuallyReceivesAllExactlyOnce(t *testing.T) {
	r := newRig(t)

	const dead = 300
	want := map[values.ServiceName]struct{}{}
	for i := range dead {
		name := "dead-" + itoa(i)
		r.add(name, newFake(false))
		want[values.ServiceName(name)] = struct{}{}
	}
	r.run()

	time.Sleep(quietWindow) // consumer asleep; channel saturated

	seen := map[values.ServiceName]int{}
	for len(seen) < dead {
		ev, _ := r.recv()
		seen[ev.Handle.Service]++
	}
	for svc, n := range seen {
		if n != 1 {
			t.Errorf("%s delivered %d times", svc, n)
		}
		delete(want, svc)
	}
	if len(want) != 0 {
		t.Errorf("lost events for %d services", len(want))
	}
	r.expectQuiet("after all delivered")
}

// ---------- resilience ----------

func TestSupervisor_InspectErrorDoesNotAffectOtherServices(t *testing.T) {
	r := newRig(t)
	broken := newFake(true)
	broken.failInspect(errors.New("boom"))
	r.add("broken", broken)

	dying := newFake(true)
	r.add("dying", dying)
	r.run()

	dying.kill(9)
	ev, _ := r.recv()
	if ev.Handle.Service != "dying" {
		t.Fatalf("got event for %q", ev.Handle.Service)
	}

	// Do not assert immediately: wait until broken is inspected again.
	base := broken.inspects.Load()
	eventually(t, func() bool { return broken.inspects.Load() >= base+2 },
		"broken service must keep being inspected after other services' events")
}

// An Inspect error is NOT evidence of death: no event.
func TestSupervisor_InspectErrorIsNotTreatedAsExit(t *testing.T) {
	r := newRig(t)
	h := newFake(true)
	h.failInspect(errors.New("transient"))
	r.add("svc", h)
	r.run()

	r.expectQuiet("inspect error")
	if _, ok := r.reg.Get(r.ws, "svc"); !ok {
		t.Error("service removed because of an inspect error")
	}
}

// After a transient error clears, a real exit is still detected.
func TestSupervisor_RecoversAfterInspectError(t *testing.T) {
	r := newRig(t)
	h := newFake(true)
	h.failInspect(errors.New("transient"))
	r.add("svc", h)
	r.run()
	r.expectQuiet("while failing")

	h.inspErr.Store(nil)
	h.kill(5)
	ev, _ := r.recv()
	if ev.ExitCode == nil || *ev.ExitCode != 5 {
		t.Errorf("exit code=%v", ev.ExitCode)
	}
}

func TestSupervisor_SlowInspectDoesNotHangShutdown(t *testing.T) {
	r := newRig(t)
	r.add("slow", &blockingHandle{release: make(chan struct{})})
	cancel := r.run()

	time.Sleep(10 * time.Millisecond) // let a tick enter Inspect
	cancel()
	// run()'s cleanup asserts Run returns; Inspect receives ctx and must honour it.
}

// blockingHandle blocks in Inspect until ctx is cancelled.
type blockingHandle struct{ release chan struct{} }

func (b *blockingHandle) ID() string                                { return "blocking" }
func (b *blockingHandle) Stop(context.Context, time.Duration) error { return nil }
func (b *blockingHandle) Inspect(ctx context.Context) (runtime.Status, error) {
	select {
	case <-ctx.Done():
		return runtime.Status{}, ctx.Err()
	case <-b.release:
		return runtime.Status{Alive: true}, nil
	}
}

// ---------- cancellation ----------

func TestSupervisor_RunReturnsOnContextCancel(t *testing.T) {
	r := newRig(t)
	r.add("svc", newFake(true))
	cancel := r.run()
	time.Sleep(10 * time.Millisecond)
	cancel() // cleanup in run() asserts prompt return
}

func TestSupervisor_RunReturnsWhenCancelledWhileDeliveryBlocked(t *testing.T) {
	r := newRig(t)
	for i := range 512 {
		r.add("dead-"+itoa(i), newFake(false))
	}
	cancel := r.run()
	time.Sleep(quietWindow) // channel saturated, delivery pending
	cancel()                // must not deadlock on a full channel
}

func TestSupervisor_AlreadyCancelledContextReturnsImmediately(t *testing.T) {
	r := newRig(t)
	r.add("svc", newFake(false))
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	done := make(chan struct{})
	go func() { defer close(done); r.sup.Run(ctx) }()
	select {
	case <-done:
	case <-time.After(waitFor):
		t.Fatal("Run with a cancelled context did not return")
	}
}

// ---------- concurrency (-race) ----------

func TestSupervisor_ConcurrentRegisterRemoveWhileRunning(t *testing.T) {
	r := newRig(t)
	r.run()

	// Drain events so delivery never blocks this test.
	stop := make(chan struct{})
	var drained atomic.Int64
	var drainWG sync.WaitGroup
	drainWG.Add(1)
	go func() {
		defer drainWG.Done()
		for {
			select {
			case <-r.sup.Events():
				drained.Add(1)
			case <-stop:
				return
			}
		}
	}()

	var wg sync.WaitGroup
	for w := range 8 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := range 100 {
				h := newFake(i%2 == 0)
				sh := registry.ServiceHandle{
					Workspace: r.ws,
					Service:   values.ServiceName("w" + itoa(w) + "-" + itoa(i)),
					Handle:    h,
				}
				_ = r.reg.Register(sh)
				h.kill(1)
				_ = r.reg.Remove(sh)
			}
		}()
	}
	wg.Wait()
	close(stop)
	drainWG.Wait()
}

func TestSupervisor_ManyWorkspacesAreIndependent(t *testing.T) {
	r := newRig(t)
	const n = 20
	handles := make([]*fakeHandle, n)
	for i := range n {
		handles[i] = newFake(true)
		_ = r.reg.Register(registry.ServiceHandle{
			Workspace: values.NewWorkspaceId(),
			Service:   "svc", // same name in every workspace
			Handle:    handles[i],
		})
	}
	r.run()

	handles[7].kill(2)
	ev, _ := r.recv()
	if ev.Handle.Handle != runtimeHandle(handles[7]) {
		t.Fatal("event attributed to the wrong workspace's instance")
	}
	r.expectQuiet("other workspaces")
}

// ---------- tiny helper ----------

func itoa(i int) string {
	const digits = "0123456789"
	if i == 0 {
		return "0"
	}
	var b [20]byte
	pos := len(b)
	for i > 0 {
		pos--
		b[pos] = digits[i%10]
		i /= 10
	}
	return string(b[pos:])
}
