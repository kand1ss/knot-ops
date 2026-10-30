package phase

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain/task"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/registry"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/supervisor/runtime"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// failsafe only: never used for synchronization, only to turn a hang into a failure.
const failsafe = 10 * time.Second

// ---------- fakes ----------

type journal struct {
	mu      sync.Mutex
	entries []string
}

func (j *journal) add(s string) { j.mu.Lock(); j.entries = append(j.entries, s); j.mu.Unlock() }
func (j *journal) all() []string {
	j.mu.Lock()
	defer j.mu.Unlock()
	return append([]string(nil), j.entries...)
}
func (j *journal) first(prefix string) int { return j.find(prefix, false) }
func (j *journal) last(prefix string) int  { return j.find(prefix, true) }
func (j *journal) find(prefix string, last bool) int {
	e := j.all()
	idx := -1
	for i, s := range e {
		if len(s) >= len(prefix) && s[:len(prefix)] == prefix {
			idx = i
			if !last {
				return i
			}
		}
	}
	return idx
}

type fakeHandle struct {
	runtime.RunHandle // unimplemented methods panic if touched
	name              values.ServiceName
	j                 *journal
	stopErr           error
	stops             atomic.Int32
}

func (h *fakeHandle) ID() string { return "pid-" + string(h.name) }
func (h *fakeHandle) Stop(context.Context, time.Duration) error {
	h.stops.Add(1)
	h.j.add("stop:" + string(h.name))
	return h.stopErr
}

type fakeRegistry struct {
	registry.RuntimeRegistry
	mu          sync.Mutex
	items       map[values.ServiceName]registry.ServiceHandle
	registerErr map[values.ServiceName]error
	removeErr   map[values.ServiceName]error
}

func newFakeRegistry() *fakeRegistry {
	return &fakeRegistry{
		items:       map[values.ServiceName]registry.ServiceHandle{},
		registerErr: map[values.ServiceName]error{},
		removeErr:   map[values.ServiceName]error{},
	}
}

func (r *fakeRegistry) Get(_ values.WorkspaceID, n values.ServiceName) (registry.ServiceHandle, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	h, ok := r.items[n]
	return h, ok
}
func (r *fakeRegistry) Register(h registry.ServiceHandle) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if err := r.registerErr[h.Service]; err != nil {
		return err
	}
	r.items[h.Service] = h
	return nil
}
func (r *fakeRegistry) Remove(_ values.WorkspaceID, n values.ServiceName) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if err := r.removeErr[n]; err != nil {
		return err
	}
	delete(r.items, n)
	return nil
}
func (r *fakeRegistry) has(n values.ServiceName) bool {
	_, ok := r.Get(values.NewWorkspaceId(), n)
	return ok
}
func (r *fakeRegistry) size() int { r.mu.Lock(); defer r.mu.Unlock(); return len(r.items) }

type fakeRuntime struct {
	runtime.Runtime
	j         *journal
	mu        sync.Mutex
	started   map[values.ServiceName]*fakeHandle
	failStart map[values.ServiceName]error
	onStart   func(domain.ServiceSpec) error // optional hook, set before Execute
}

func (r *fakeRuntime) Start(_ context.Context, spec domain.ServiceSpec) (runtime.RunHandle, error) {
	r.j.add("start:" + string(spec.Name))
	if r.onStart != nil {
		if err := r.onStart(spec); err != nil {
			return nil, err
		}
	}
	if err := r.failStart[spec.Name]; err != nil {
		return nil, err
	}
	h := &fakeHandle{name: spec.Name, j: r.j}
	r.mu.Lock()
	r.started[spec.Name] = h
	r.mu.Unlock()
	return h, nil
}
func (r *fakeRuntime) startedHandle(n values.ServiceName) *fakeHandle {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.started[n]
}

// ---------- harness ----------

type snapshot struct{ Started, Stopped, Skipped, Failed uint32 }

func (snapshot) IsTaskEvent() {}
func snap(st *Stats) task.TaskEvent {
	return snapshot{st.started.Load(), st.stopped.Load(), st.skipped.Load(), st.failed.Load()}
}

type harness struct {
	t            *testing.T
	taskID       values.TaskID
	wsID         values.WorkspaceID
	lock         *core.WorkspaceLock
	reg          *fakeRegistry
	rt           *fakeRuntime
	j            *journal
	runner       Runner
	execCtx      context.Context
	execCancel   context.CancelFunc
	callerCtx    context.Context
	callerCancel context.CancelFunc
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	j := &journal{}
	reg := newFakeRegistry()
	rt := &fakeRuntime{j: j, started: map[values.ServiceName]*fakeHandle{}, failStart: map[values.ServiceName]error{}}
	lock := core.NewWorkspaceLock()
	h := &harness{t: t, taskID: values.NewTaskID(), wsID: values.NewWorkspaceId(), lock: lock, reg: reg, rt: rt, j: j,
		runner: NewRunner(nil, reg, lock, rt)}
	h.execCtx, h.execCancel = context.WithCancel(context.Background())
	h.callerCtx, h.callerCancel = context.WithCancel(context.Background())
	t.Cleanup(func() { h.execCancel(); h.callerCancel() })
	return h
}

// seed registers already-running services (as if started by a previous `up`).
func (h *harness) seed(names ...values.ServiceName) map[values.ServiceName]*fakeHandle {
	out := map[values.ServiceName]*fakeHandle{}
	for _, n := range names {
		fh := &fakeHandle{name: n, j: h.j}
		out[n] = fh
		_ = h.reg.Register(registry.ServiceHandle{Workspace: h.wsID, Service: n, Handle: fh})
	}
	return out
}

// execute runs the runner exactly like the session does and drains all events.
func (h *harness) execute(phases []Phase) []task.TaskEvent {
	h.t.Helper()
	h.lock.Lock(h.wsID) // Execute owns the unlock
	out := make(chan task.TaskEvent)
	go h.runner.Execute(h.execCtx, h.callerCtx, h.execCancel, h.taskID, h.wsID, phases, out, snap)

	var evs []task.TaskEvent
	deadline := time.After(failsafe)
	for {
		select {
		case ev, ok := <-out:
			if !ok {
				h.assertLockReleased()
				return evs
			}
			evs = append(evs, ev)
		case <-deadline:
			h.t.Fatalf("Execute did not close the channel in %s; events so far: %d", failsafe, len(evs))
		}
	}
}

func (h *harness) assertLockReleased() {
	h.t.Helper()
	ok := make(chan struct{})
	go func() { h.lock.Lock(h.wsID); h.lock.Unlock(h.wsID); close(ok) }()
	select {
	case <-ok:
	case <-time.After(failsafe):
		h.t.Fatal("workspace lock was not released")
	}
}

// ---------- phase builders & event helpers ----------

func spec(name values.ServiceName, depends ...values.ServiceName) domain.ServiceSpec {
	return domain.ServiceSpec{Name: values.ServiceName(name), Command: "run-" + string(name), Depends: depends}
}

func startPhase(dependencies map[values.ServiceName][]values.ServiceName, waves ...[]values.ServiceName) Phase {
	p := Phase{kind: phaseKindStart}
	for _, w := range waves {
		var units []PhaseUnit
		for _, n := range w {
			units = append(units, PhaseUnit{id: values.NewTaskID(), service: values.ServiceName(n), spec: spec(n, dependencies[n]...)})
		}
		p.waves = append(p.waves, units)
	}
	return p
}

func stopPhase(dependencies map[values.ServiceName][]values.ServiceName, waves ...[]values.ServiceName) Phase {
	p := startPhase(dependencies, waves...)
	p.kind = phaseKindStop
	for i := range p.waves {
		for j := range p.waves[i] {
			p.waves[i][j].spec = domain.ServiceSpec{} // orphans/stop units carry no spec
		}
	}
	return p
}

func lastDone(t *testing.T, evs []task.TaskEvent) task.TaskDone {
	t.Helper()
	if len(evs) == 0 {
		t.Fatal("no events")
	}
	s, ok := evs[len(evs)-1].(task.TaskDone)
	if !ok {
		t.Fatalf("last event is %T, want TaskDone", evs[len(evs)-1])
	}
	return s
}

func serviceEvents(evs []task.TaskEvent, name string) []task.Event {
	var out []task.Event
	for _, ev := range evs {
		if se, ok := ev.(task.ServiceEvent); ok && se.Service == name {
			out = append(out, se.Event)
		}
	}
	return out
}

func terminalOf(evs []task.TaskEvent, name string) task.Event {
	var term task.Event
	for _, e := range serviceEvents(evs, name) {
		switch e.(type) {
		case task.RunningEvent, task.StoppedEvent, task.SkippedEvent, task.FailedEvent:
			term = e
		}
	}
	return term
}

// Invariant: every planned unit ends in exactly one terminal state, none stay "pending".
func assertOneTerminalPerUnit(t *testing.T, phases []Phase, evs []task.TaskEvent) {
	t.Helper()
	counts := map[values.TaskID]int{}
	for _, ev := range evs {
		switch e := ev.(type) {
		case task.ServiceEvent:
			switch e.Event.(type) {
			case task.RunningEvent, task.StoppedEvent, task.SkippedEvent, task.FailedEvent:
				counts[e.TaskID]++
			}
		case task.TaskCancelled:
			counts[e.TaskID]++
		}
	}
	for _, p := range phases {
		for _, w := range p.waves {
			for _, u := range w {
				if counts[u.id] != 1 {
					t.Errorf("unit %q has %d terminal events, want 1", u.service, counts[u.id])
				}
			}
		}
	}
}

func assertPlanFirst(t *testing.T, evs []task.TaskEvent, want ...values.ServiceName) {
	t.Helper()
	plan, ok := evs[0].(task.TaskAccepted)
	if !ok {
		t.Fatalf("first event is %T, want TaskAccepted", evs[0])
	}
	if len(plan.Services) != len(want) {
		t.Fatalf("plan services = %v, want %v", plan.Services, want)
	}
	for i := range want {
		if plan.Services[i] != want[i] {
			t.Fatalf("plan services = %v, want %v", plan.Services, want)
		}
	}
}

// ---------- happy paths ----------

func TestExecute_StartSingleWave(t *testing.T) {
	h := newHarness(t)
	phases := []Phase{startPhase(nil, []values.ServiceName{"a", "b"})}

	evs := h.execute(phases)

	assertPlanFirst(t, evs, "a", "b")
	if s := lastDone(t, evs); s != (task.TaskDone{TaskID: s.TaskID, Started: 2}) {
		t.Fatalf("stats = %+v, want Started=2", s)
	}
	for _, n := range []string{"a", "b"} {
		if !h.reg.has(values.ServiceName(n)) {
			t.Errorf("%s not registered", n)
		}
		if _, ok := terminalOf(evs, n).(task.RunningEvent); !ok {
			t.Errorf("%s terminal = %T, want RunningEvent", n, terminalOf(evs, n))
		}
	}
	assertOneTerminalPerUnit(t, phases, evs)
}

func TestExecute_WaveBarrier_NextWaveSeesAllPreviousRegistered(t *testing.T) {
	h := newHarness(t)
	var violations atomic.Int32
	h.rt.onStart = func(s domain.ServiceSpec) error {
		// wave 2 unit may start only after the WHOLE wave 1 is registered.
		if s.Name == "api" && (!h.reg.has("db") || !h.reg.has("cache")) {
			violations.Add(1)
		}
		return nil
	}

	evs := h.execute([]Phase{startPhase(nil, []values.ServiceName{"db", "cache"}, []values.ServiceName{"api"})})

	if v := violations.Load(); v != 0 {
		t.Fatalf("wave barrier violated: next wave started before previous completed")
	}
	if s := lastDone(t, evs); s.Started != 3 {
		t.Fatalf("Started = %d, want 3", s.Started)
	}
}

func TestExecute_PruneCompletesBeforeAnyStart(t *testing.T) {
	h := newHarness(t)
	h.seed("orphan-a", "orphan-b")
	phases := []Phase{stopPhase(nil, []values.ServiceName{"orphan-a", "orphan-b"}), startPhase(nil, []values.ServiceName{"api"})}

	evs := h.execute(phases)

	if h.j.last("stop:") > h.j.first("start:") || h.j.first("start:") == -1 {
		t.Fatalf("prune must fully precede start; journal: %v", h.j.all())
	}
	s := lastDone(t, evs)
	if s.Stopped != 2 || s.Started != 1 || s.Failed != 0 {
		t.Fatalf("stats = %+v, want Stopped=2 Started=1", s)
	}
	assertOneTerminalPerUnit(t, phases, evs)
}

func TestExecute_DownStopsDependentsFirst(t *testing.T) {
	h := newHarness(t)
	handles := h.seed("api", "db")

	evs := h.execute([]Phase{stopPhase(nil, []values.ServiceName{"api"}, []values.ServiceName{"db"})})

	if h.j.first("stop:api") > h.j.first("stop:db") {
		t.Fatalf("dependent must stop before dependency; journal: %v", h.j.all())
	}
	if h.reg.size() != 0 {
		t.Fatalf("registry must be empty after down, has %d", h.reg.size())
	}
	for n, hd := range handles {
		if hd.stops.Load() != 1 {
			t.Errorf("%s stopped %d times, want 1", n, hd.stops.Load())
		}
	}
	if s := lastDone(t, evs); s.Stopped != 2 {
		t.Fatalf("Stopped = %d, want 2", s.Stopped)
	}
}

// ---------- fail paths ----------

func TestExecute_StartFailure_DoesNotAbortSiblingsOrNextWave(t *testing.T) {
	h := newHarness(t)
	h.rt.failStart["bad"] = errors.New("exec: not found")
	deps := make(map[values.ServiceName][]values.ServiceName)
	deps[values.ServiceName("later")] = []values.ServiceName{values.ServiceName("bad")}

	phases := []Phase{startPhase(deps, []values.ServiceName{"bad", "good"}, []values.ServiceName{"later"})}

	evs := h.execute(phases)

	if _, ok := terminalOf(evs, "bad").(task.FailedEvent); !ok {
		t.Fatalf("bad terminal = %T, want FailedEvent", terminalOf(evs, "bad"))
	}
	if !h.reg.has("good") {
		t.Error("independent sibling must still start")
	}

	if h.reg.has("later") {
		t.Error("dependent service must not start after dependency failure")
	}

	if _, ok := terminalOf(evs, "later").(task.SkippedEvent); !ok {
		t.Errorf(
			"later terminal = %T, want SkippedEvent",
			terminalOf(evs, "later"),
		)
	}

	if s := lastDone(t, evs); s.Failed != 1 || s.Started != 1 || s.Skipped != 1 {
		t.Fatalf(
			"stats = %+v, want Failed=1 Started=1 Skipped=1",
			s,
		)
	}

	assertOneTerminalPerUnit(t, phases, evs)
}

func TestExecute_RegisterFailure_StopsSpawnedProcess(t *testing.T) {
	h := newHarness(t)
	h.reg.registerErr["api"] = errors.New("already registered")

	evs := h.execute([]Phase{startPhase(nil, []values.ServiceName{"api"})})

	hd := h.rt.startedHandle("api")
	if hd == nil || hd.stops.Load() != 1 {
		t.Fatalf("spawned-but-untracked process must be stopped exactly once (leak guard)")
	}
	if _, ok := terminalOf(evs, "api").(task.FailedEvent); !ok {
		t.Fatalf("terminal = %T, want FailedEvent", terminalOf(evs, "api"))
	}
	if s := lastDone(t, evs); s.Failed != 1 || s.Started != 0 {
		t.Fatalf("stats = %+v", s)
	}
}

func TestExecute_StopFailure_KeepsRegistryEntry(t *testing.T) {
	h := newHarness(t)
	hd := h.seed("db")["db"]
	hd.stopErr = errors.New("permission denied")

	evs := h.execute([]Phase{stopPhase(nil, []values.ServiceName{"db"})})

	if !h.reg.has("db") {
		t.Error("service whose stop failed must remain tracked (process may still run)")
	}
	if s := lastDone(t, evs); s.Failed != 1 || s.Stopped != 0 {
		t.Fatalf("stats = %+v", s)
	}
}

func TestExecute_RemoveFailure_DoesNotRestopProcess(t *testing.T) {
	h := newHarness(t)
	hd := h.seed("db")["db"]
	h.reg.removeErr["db"] = errors.New("registry corrupted")

	evs := h.execute([]Phase{stopPhase(nil, []values.ServiceName{"db"})})

	if hd.stops.Load() != 1 {
		t.Fatalf("Stop called %d times, want exactly 1 (no re-stop after Remove failure)", hd.stops.Load())
	}
	if _, ok := terminalOf(evs, "db").(task.FailedEvent); !ok {
		t.Fatalf("terminal = %T, want FailedEvent", terminalOf(evs, "db"))
	}
}

func TestExecute_OrphanStopFailureDoesNotBlockStartPhase(t *testing.T) {
	h := newHarness(t)
	h.seed("orphan")["orphan"].stopErr = errors.New("boom")

	evs := h.execute([]Phase{stopPhase(nil, []values.ServiceName{"orphan"}), startPhase(nil, []values.ServiceName{"api"})})

	if s := lastDone(t, evs); s.Failed != 1 || s.Started != 1 {
		t.Fatalf("stats = %+v, want Failed=1 Started=1", s)
	}
}

// ---------- edge cases ----------

func TestExecute_AlreadyRunningIsSkipped(t *testing.T) {
	h := newHarness(t)
	h.seed("api")

	evs := h.execute([]Phase{startPhase(nil, []values.ServiceName{"api"})})

	if h.j.first("start:") != -1 {
		t.Error("Runtime.Start must not be called for an already running service")
	}
	if _, ok := terminalOf(evs, "api").(task.SkippedEvent); !ok {
		t.Fatalf("terminal = %T, want SkippedEvent", terminalOf(evs, "api"))
	}
	if s := lastDone(t, evs); s.Skipped != 1 || s.Failed != 0 {
		t.Fatalf("stats = %+v", s)
	}
}

func TestExecute_StopOfNotRunningIsSkippedNotFailed(t *testing.T) {
	h := newHarness(t)

	evs := h.execute([]Phase{stopPhase(nil, []values.ServiceName{"ghost"})})

	if _, ok := terminalOf(evs, "ghost").(task.SkippedEvent); !ok {
		t.Fatalf("terminal = %T, want SkippedEvent", terminalOf(evs, "ghost"))
	}
	if s := lastDone(t, evs); s.Failed != 0 || s.Skipped != 1 {
		t.Fatalf("stats = %+v", s)
	}
}

func TestExecute_NoPhases_EmitsPlanAndDoneThenCloses(t *testing.T) {
	h := newHarness(t)

	evs := h.execute(nil)

	if len(evs) != 2 {
		t.Fatalf("events = %d, want plan + done", len(evs))
	}
	assertPlanFirst(t, evs)
	if s := lastDone(t, evs); s != (task.TaskDone{TaskID: s.TaskID}) {
		t.Fatalf("stats = %+v, want zero", s)
	}
}

func TestExecute_EmptyWaveDoesNotBreakExecution(t *testing.T) {
	h := newHarness(t)
	p := startPhase(nil, []values.ServiceName{"a"})
	p.waves = append([][]PhaseUnit{{}}, p.waves...)

	evs := h.execute([]Phase{p})

	if s := lastDone(t, evs); s.Started != 1 {
		t.Fatalf("Started = %d, want 1", s.Started)
	}
}

func TestExecute_CancelledBeforeStart_AllUnitsReportCancelled(t *testing.T) {
	h := newHarness(t)
	phases := []Phase{startPhase(nil, []values.ServiceName{"a", "b"}, []values.ServiceName{"c"})}
	h.execCancel() // consumer (callerCtx) stays alive: Cancelled events must be delivered

	evs := h.execute(phases)

	if h.j.first("start:") != -1 {
		t.Error("nothing may be started after cancellation")
	}
	assertOneTerminalPerUnit(t, phases, evs)
	_ = lastDone(t, evs) // done is still emitted
}

func TestExecute_CancelBetweenWaves_RemainingUnitsCancelled(t *testing.T) {
	h := newHarness(t)
	h.rt.onStart = func(s domain.ServiceSpec) error {
		if s.Name == "first" {
			h.execCancel() // deterministic: fires inside wave 1, before wave 2 begins
		}
		return nil
	}
	phases := []Phase{startPhase(nil, []values.ServiceName{"first"}, []values.ServiceName{"second", "third"})}

	evs := h.execute(phases)

	if h.j.first("start:second") != -1 || h.j.first("start:third") != -1 {
		t.Errorf("wave 2 must not start after cancel; journal: %v", h.j.all())
	}
	assertOneTerminalPerUnit(t, phases, evs)
}

func TestExecute_ConsumerGone_DoesNotLeakGoroutineOrLock(t *testing.T) {
	h := newHarness(t)
	h.lock.Lock(h.wsID)
	out := make(chan task.TaskEvent) // unbuffered and never read
	h.callerCancel()

	finished := make(chan struct{})
	go func() {
		defer close(finished)
		h.runner.Execute(h.execCtx, h.callerCtx, h.execCancel, h.taskID, h.wsID,
			[]Phase{startPhase(nil, []values.ServiceName{"a", "b"}, []values.ServiceName{"c"})}, out, snap)
	}()

	select {
	case <-finished:
	case <-time.After(failsafe):
		t.Fatal("Execute blocked on a dead consumer")
	}
	h.assertLockReleased()
}

// ---------- concurrency ----------

func TestExecute_UnitsInWaveRunConcurrently(t *testing.T) {
	const n = 8
	h := newHarness(t)

	var entered sync.WaitGroup
	entered.Add(n)
	release := make(chan struct{})
	go func() { entered.Wait(); close(release) }()

	// Rendezvous: every unit blocks until ALL n have entered Start.
	// A serialized implementation can never pass this.
	h.rt.onStart = func(domain.ServiceSpec) error {
		entered.Done()
		select {
		case <-release:
			return nil
		case <-time.After(3 * time.Second):
			return errors.New("units are serialized")
		}
	}
	names := make([]values.ServiceName, n)
	for i := range names {
		names[i] = values.ServiceName('a' + rune(i))
	}

	evs := h.execute([]Phase{startPhase(nil, names)})

	if s := lastDone(t, evs); s.Started != n || s.Failed != 0 {
		t.Fatalf("stats = %+v, want Started=%d (units must run concurrently)", s, n)
	}
}

func TestExecute_ManyUnits_CountersAndEventsConsistent(t *testing.T) {
	const n = 64
	h := newHarness(t)
	names := make([]values.ServiceName, n)
	for i := range names {
		names[i] = values.ServiceName("svc-" + string(rune('A'+i/26)) + string(rune('a'+i%26)))
	}
	phases := []Phase{startPhase(nil, names[:n/2], names[n/2:])}

	evs := h.execute(phases)

	if s := lastDone(t, evs); s.Started != n || s.Failed != 0 {
		t.Fatalf("stats = %+v, want Started=%d", s, n)
	}
	if h.reg.size() != n {
		t.Fatalf("registry size = %d, want %d", h.reg.size(), n)
	}
	assertOneTerminalPerUnit(t, phases, evs)
	assertPlanFirst(t, evs[:1], func() []values.ServiceName {
		out := make([]values.ServiceName, n)
		for i, nm := range names {
			out[i] = values.ServiceName(nm)
		}
		return out
	}()...)
}

func TestExecute_ConcurrentMixedOutcomes_CountersExact(t *testing.T) {
	h := newHarness(t)
	for i := 0; i < 10; i++ {
		h.rt.failStart[values.ServiceName("bad"+string(rune('0'+i)))] = errors.New("x")
	}
	h.seed("run0", "run1", "run2")
	var wave []values.ServiceName
	for i := 0; i < 10; i++ {
		wave = append(wave, values.ServiceName("bad"+string(rune('0'+i))), values.ServiceName("ok"+string(rune('0'+i))))
	}
	wave = append(wave, "run0", "run1", "run2") // already running -> skipped
	phases := []Phase{startPhase(nil, wave)}

	evs := h.execute(phases)

	if s := lastDone(t, evs); s != (task.TaskDone{TaskID: s.TaskID, Started: 10, Skipped: 3, Failed: 10}) {
		t.Fatalf("stats = %+v, want Started=10 Skipped=3 Failed=10", s)
	}
	assertOneTerminalPerUnit(t, phases, evs)
}
