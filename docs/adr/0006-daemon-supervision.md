# 0006 — Daemon supervision: user-session launcher vs. system-managed unit

## Status
Accepted

## Context
ADR-0001 defines a host-wide daemon with a privileged system endpoint
(`/run/knot/knot.sock`, owner `root:knot`, mode `0660`) for production use.
The current `KnotClient::connect_or_launch` unconditionally spawns the
daemon via `Command::spawn` when a health check fails, regardless of which
socket path is targeted. This lets an unprivileged CLI invocation launch a
process expected to hold system-level access — contradicting the placement
and privilege model ADR-0001 already established, and reintroducing the
per-invocation PID/zombie-tracking problems documented in [issue/discussion ref].

## Decision
Split daemon lifecycle ownership into two explicit, non-interchangeable modes,
selected by socket path:

- **UserSession** (`$XDG_RUNTIME_DIR/knot/knot.sock`): unprivileged,
  per-user daemon. The CLI retains full ownership of spawn/health/restart —
  current `DaemonLauncher` behavior, unchanged.
- **SystemManaged** (`/run/knot/knot.sock`): the CLI never spawns this
  daemon. It only connects and reports actionable status. Lifecycle is
  owned exclusively by the platform service manager (systemd/launchd/SCM),
  installed via `knot daemon install` (roadmap v0.5).

Mode is derived deterministically from socket path prefix — no runtime
flag, no environment-dependent heuristic.

## Consequences
- `DaemonLauncher` trait scope narrows to UserSession only; its
  implementors (`ExternalPathLauncher`, `CurrentExeLauncher`,
  `SystemPathLauncher`) must not be reachable from the SystemManaged path.
- `knot daemon install` becomes a required v0.5 deliverable, not optional
  polish — SystemManaged mode is unusable without it.
- Go/Python SDKs must replicate the same mode-detection logic; this ADR is
  their normative reference, not the Rust implementation.
- `HealthcheckError::ZombieProcess`/`ProcessNotExists` variants become
  dead code once UserSession fully migrates to the lock-based model from
  [prior discussion] — tracked as follow-up cleanup, not blocking this ADR.