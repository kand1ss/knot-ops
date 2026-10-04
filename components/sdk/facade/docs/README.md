# Knot Client Architecture (`knot-client`)

`knot-client` is the Rust client library for talking to the `knot` background daemon. It hides daemon lifecycle
management, workspace state evaluation, and streamed command execution behind a **typestate / handle pattern**: every
phase of the connection is a distinct type, and invalid operations do not compile.

> This document describes **concepts and invariants**, not the API surface. For signatures, fields, and defaults, see
> the rustdoc on the corresponding types. If this document and the code disagree on a detail, the code wins.

---

## 1. Design Rationale

Talking to a local daemon involves many partial states: no socket, a dead process with leftover lock files, a hung
process, an unsynchronized workspace. A monolithic client would check these at runtime and return "invalid state" errors.

Instead, each state is a dedicated **handle**. Handle methods consume `self` and return the handle of the next state,
so only operations that are valid in the current state are callable.

**Trade-off:** the caller must handle state transitions explicitly (pattern matching / chaining) in exchange for
compile-time safety, discoverable APIs, and no internal state flags or locks.

---

## 2. Lifecycle

Connecting is a two-stage process:

1. **Daemon discovery** — `KnotClient::connect()` inspects the runtime artifacts (lock file, IPC endpoint), the
   OS process table, and endpoint responsiveness, and classifies the daemon into a `ConnectState`.
2. **Workspace handshake** — once connected, the client sends workspace identity and manifest; the daemon answers
   whether the workspace is in sync, producing a `DaemonSession`.

Daemon connection is intentionally decoupled from workspace context: workspace data is only needed at the handshake.

Diagrams: [flow.md](flow.md) (decision tree), [states.md](states.md) (state machine).

---

## 3. Handles

| Phase       | Handle            | Meaning                                                   | Transitions to                        |
|:------------|:------------------|:----------------------------------------------------------|:--------------------------------------|
| Discovery   | `OfflineHandle`   | Daemon is not running.                                    | `ConnectedHandle` (via `launch`)      |
| Discovery   | `StaleHandle`     | Leftover artifacts of a dead daemon.                      | `OfflineHandle` (via `clean`)         |
| Discovery   | `KillHandle`      | A matching daemon process exists but is unresponsive.     | `StaleHandle` (via `kill`)            |
| Discovery   | `ConnectedHandle` | IPC channel is healthy; workspace not yet checked.        | `DaemonSession` (via `handshake`)     |
| Session     | `UncommittedHandle` | Workspace is registered but out of sync with the daemon. | `ControlHandle` (via `sync`)          |
| Session     | `ControlHandle`   | Workspace is in sync; the main operational handle.        | `TaskHandle` (via `up`/`down`/`sync`) |
| Execution   | `TaskHandle<E>`   | A running long-lived command, exposed as an event stream. | back to `ControlHandle` when finished |

Key properties:

* **Recovery is explicit.** Stale and hung daemons are never fixed silently; each recovery step is a separate,
  visible transition.
* **Process identity is verified**, not assumed. A PID alone is not trusted to identify the daemon (PID reuse).
* **Request/response vs. streaming.** Short queries (e.g. status) return directly; long operations (`up`, `down`,
  `sync`) return a `TaskHandle` that streams progress events and supports cancellation.

---

## 4. Transport

* gRPC over local IPC (Unix domain sockets / Windows named pipes), provided by the `knot-grpc` crate.
* Every streamed command is identified by an ID returned in response metadata; this ID correlates events and
  cancellation requests.
* Timeouts are governed by a policy that distinguishes short RPCs from long-running streams.

---

## 5. Errors

All failures surface as a single `ClientError`, grouped by origin:

* **Workspace** — workspace missing or its data unreadable.
* **Daemon lifecycle** — launching or controlling the daemon process failed.
* **Protocol / Transport** — gRPC status errors and channel failures.
* **Contract** — the daemon violated an expected protocol invariant (e.g. version mismatch).
* **I/O** — filesystem and OS errors.

Errors are designed to carry user-facing context and a suggested remedy so the CLI can render them directly.

---

## 6. Usage

A complete, compiled end-to-end example lives in the crate's `examples/` directory and in the CLI, which is the
reference consumer. Typical flow:

1. `connect` → resolve the `ConnectState` until you hold a `ConnectedHandle`.
2. `handshake` → obtain a `ControlHandle`, synchronizing first if the daemon reports drift.
3. Run commands and consume the resulting `TaskHandle` event stream; cancel it if needed.