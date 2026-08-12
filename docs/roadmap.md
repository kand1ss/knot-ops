# Roadmap

> Each version delivers a working tool. No version leaves the project in a
> broken state.

## Product Direction

Knot is a local **service orchestrator**. Process supervisors orchestrate
individual processes, and container orchestrators orchestrate containers.
Knot orchestrates services: their lifecycle, dependencies, runtime state, and
connectivity.

A service is an abstract unit managed by a runtime driver. The first driver is
`ProcessRuntime`; `DockerRuntime` follows as another implementation of the
same lifecycle contract. Runtimes are implementation details of a service, not
the core product boundary.

Knot also provides a local service-connectivity plane. A service name such as
`http://backend.knot:8000` is resolved and routed to the active IPv4 endpoint
registered for `backend`, regardless of whether that endpoint is supplied by a
local process or a container. This capability is developed as local DNS name
resolution plus a routing proxy.

External SDKs, hooks, and third-party integrations are not near-term product
goals. They may be considered only after the service model, runtime contract,
and connectivity plane are stable.

---

## v0.1 — Service Orchestration Foundation

**Goal: a minimal, working daemon and CLI managing process-backed services,
with a stable workspace and hashing model from day one.**

### Daemon

- Global (user-session) daemon and local IPC endpoint
- Workspace registration with immutable `workspace_id`
- `Handshake` for daemon reachability, workspace registration, and cheap
  configuration-drift detection before workspace-scoped commands
- `Sync` for detailed configuration comparison and daemon-state reconciliation
- TOML configuration parsing and canonical configuration hashes
- Service DAG engine for dependency ordering
- `ProcessRuntime` for local process lifecycle management
- In-memory runtime state store
- Service statuses: `Stopped`, `Starting`, `Running`, `Waiting`, `Failed`
  — limited to states the v0.1 engine can actually produce.
  `Degraded`, `Restarting`, and `Stopping` are introduced in v0.2 alongside
  the restart-policy engine that generates them, not before.

### CLI

- `knot init` — initialize and register a workspace
- `knot up` — synchronize when needed, then start services
- `knot down` — stop services
- `knot ps` — list services in the current workspace and their status
- `knot inspect` — inspect state and find problems
- `knot daemon logs` — read the daemon's own operational log directly from
  disk (no IPC dependency — must work even when the daemon is unreachable)

#### Global flags

- `-d`, `--detach` — detach from the daemon and run in the background
- `--debug` — enable debug logging
- `--workspace <path>` — use a specific workspace
- `--no-color` — disable ANSI color output
- `--no-interactive` — disable interactive prompts
- `--no-sync` — disable synchronization before starting services

### Config — `[services.<service>]`

```toml
[services.backend]
runtime    = "process"
cmd        = "cargo run"
dir        = "./backend"
env        = { KEY = "value" }
env_file   = ".env"
depends_on = ["postgres"]
```

---

## v0.2 — Observability

**Goal: make what the daemon already captures visible to the user.**

### Daemon

- Log aggregation across runtimes, built on the internal capture added in
  v0.2
- Service lifecycle events surfaced through the daemon
- Diagnostics for configuration drift and failed health checks

### CLI

- `knot logs <service>` / `knot logs --all`
- `knot logs --count <n>`
- `knot logs --follow` / `-f`

---

## v0.3 — Reliability

**Goal: keep services running and recover from failure without manual
intervention, on the runtime already proven in v0.1.**

### Daemon

- Persistent runtime state store (SQLite)
- Crash recovery and runtime reattachment
- Health checks: `Command`, `Tcp`, `Http`, and `ProcessAlive`
- Restart policies: `Never`, `Always`, `OnFailure`, and `Backoff`
- Extended service statuses: `Degraded`, `Restarting`, `Stopping`
- Internal stdout/stderr capture for services, buffered per-service in the
  daemon (in-memory or on-disk). No public API yet — this exists so that
  restart-loop diagnostics aren't lost before v0.3 ships log aggregation.
  Without this, a service that fails and restarts repeatedly in v0.2 leaves
  no trace of *why*, and that gap can't be closed retroactively.

### CLI

- `knot restart`

---

## v0.4 — Multiple Runtimes

**Goal: run process-backed and container-backed services through the same
service lifecycle and connectivity model.**

### Daemon

- Stable runtime-driver contract for start, stop, inspect, and endpoint
  discovery — generalized now against two real runtimes, not designed
  speculatively ahead of the second one
- `DockerRuntime` for Docker container lifecycle management
- Runtime-specific endpoint discovery feeding the common endpoint registry
- Health checks and restart policies (v0.2) apply uniformly across both
  runtimes
- Uniform dependency handling and state transitions across runtimes

### Config

```toml
[services.postgres]
runtime = "docker"
image   = "postgres:16"
port    = 5432
```

---

## v0.5 — Network Core

**Goal: make services reachable through stable local names instead of
runtime-specific addresses, across both runtimes introduced in v0.4.**

### Daemon

- Runtime endpoint registry mapping a service to its active IPv4 target
- Workspace-scoped service-name registry and collision policy
- Local DNS resolution for the `.knot` domain
- HTTP routing proxy that maps `backend.knot` requests to the registered
  endpoint for `backend`
- Route updates when a service is started, stopped, or synchronized

### User Experience

- Reach a service through a stable address, for example
  `http://backend.knot:8000`
- Report unresolved services and unavailable runtime targets clearly

The first version targets local HTTP traffic. HTTPS termination, non-HTTP
protocols, and cross-host routing are explicitly out of scope.

---

## v0.6 — Extended Management

**Goal: operate subsets of a service graph and manage multiple workspaces,
without losing the dependency and connectivity guarantees from v0.4–v0.5.**

### CLI

- `knot up --service <name>` / `-s <name>`
- `knot up --group <name>` / `-g <name>`
- `knot down --service <name>` / `-s <name>`
- `knot down --group <name>` / `-g <name>`
- `knot daemon start`, `stop`, and `status`
- `knot workspace list` — list registered workspaces

### Config

```toml
[groups]
infra = ["postgres", "redis"]
app   = ["backend", "frontend"]
```

---

## v0.7 — CLI Improvements

**Goal: improve the day-to-day look and feel of the CLI.**

### CLI

- Improved output formatting for `ps`, `logs`, and `status`-style views
- Service endpoint and route details surfaced in `ps`
- Consistent color/interactive conventions across all commands

---

## v0.8 — Configuration and Developer Experience

**Goal: make service definitions safe to change and easy to diagnose.**

### CLI

- `knot check` — validate workspace initialization, configuration, runtimes,
  and connectivity prerequisites
- Configuration editing commands with validation

### Daemon

- Optional file watching that marks configuration as drifted
- Clear remediation guidance through `Handshake` and `Sync`

---

## Future Considerations

- A TUI dashboard (live service table, log panel, dependency/routing view)
  is a plausible future direction but is explicitly deferred past v0.8, not
  scheduled — CLI improvements (v0.7) are the near-term investment in
  usability instead.
- Additional runtimes are evaluated by their ability to implement the
  runtime contract and register service endpoints.
- Hooks, SDKs, and reusable components remain possible extensions, but must
  not define the service model or bypass the daemon's lifecycle and
  connectivity guarantees.