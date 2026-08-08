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

**Goal: manage local process-backed services with a global daemon and a
consistent workspace model.**

### Daemon

- Global daemon and local IPC endpoint
- Workspace registration with immutable `workspace_id`
- `Handshake` for daemon reachability, workspace registration, and cheap
  configuration-drift detection before workspace-scoped commands
- `Sync` for detailed configuration comparison and daemon-state reconciliation
- TOML configuration parsing and canonical configuration hashes
- Service DAG engine for dependency ordering
- `ProcessRuntime` for local process lifecycle management
- In-memory runtime state store
- Service statuses: `Stopped`, `Starting`, `Running`, `Failed`

### CLI

- `knot init` — initialize and register a workspace
- `knot up` — synchronize when needed, then start services
- `knot down` — stop services
- `knot status` — list workspace services and their statuses
- `knot ps <workspace>` - list running services in workspace

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

## v0.2 — Service Connectivity

**Goal: make services reachable through stable local names instead of
runtime-specific addresses.**

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

## v0.3 — Multiple Runtimes

**Goal: run process-backed and container-backed services through the same
service lifecycle and connectivity model.**

### Daemon

- Stable runtime-driver contract for start, stop, inspect, and endpoint
  discovery
- `DockerRuntime` for Docker container lifecycle management
- Runtime-specific endpoint discovery feeding the common endpoint registry
- Uniform dependency handling and state transitions across runtimes

### Config

```toml
[services.postgres]
runtime = "docker"
image   = "postgres:16"
port    = 5432
```

---

## v0.4 — Reliability and Recovery

**Goal: preserve service state and recover safely from daemon failures.**

### Daemon

- Persistent runtime state store (SQLite)
- Crash recovery and runtime reattachment
- Health checks: `Command`, `Tcp`, `Http`, and `ProcessAlive`
- Restart policies: `Never`, `Always`, `OnFailure`, and `Backoff`
- Extended service statuses: `Waiting`, `Degraded`, `Restarting`, and
  `Stopping`

### CLI

- `knot restart`

---

## v0.5 — Service Operations

**Goal: operate subsets of a service graph without losing dependency and
connectivity guarantees.**

### CLI

- `knot up --service <name>` / `-s <name>`
- `knot up --group <name>` / `-g <name>`
- `knot down --service <name>` / `-s <name>`
- `knot down --group <name>` / `-g <name>`
- `knot daemon start`, `stop`, and `status`

### Config

```toml
[groups]
infra = ["postgres", "redis"]
app   = ["backend", "frontend"]
```

---

## v0.6 — Observability

**Goal: make the service graph, runtime state, and connectivity decisions
observable.**

### Daemon

- Log aggregation across runtimes
- Service lifecycle and routing events
- Diagnostics for configuration drift, endpoint registration, and failed
  resolution

### CLI

- `knot logs <service>` / `knot logs --all`
- `knot logs --count <n>`
- `knot logs --follow` / `-f`
- Service endpoint and route details in `knot status`

---

## v0.7 — TUI Dashboard

**Goal: provide a live view of services, their runtimes, and connectivity.**

### TUI

- Service table: name, runtime, status, endpoint, uptime, and restarts
- Live logs panel
- Dependency and routing status
- Navigation between services

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

- Additional runtimes are evaluated by their ability to implement the runtime
  contract and register service endpoints.
- Hooks, SDKs, and reusable components remain possible extensions, but must
  not define the service model or bypass the daemon's lifecycle and
  connectivity guarantees.
