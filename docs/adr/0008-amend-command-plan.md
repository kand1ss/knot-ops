# 0008 — Service events instead of CommandPlan

## Status

Accepted

Amends [0002 — CommandPlan and unified cancellation](0002-command-plan.md).
Supersedes the **plan-then-tasks** part of 0002 (`CommandPlan`, `TaskGroup`, `TaskPlan`, legacy shared `Task*` events). The **unified cancellation** part of 0002 remains in force, with naming aligned to `task_id` (renamed from `execution_id`) and unary `CancelTask` (renamed from `CancelExecution`).

## Context

ADR 0002 introduced a plan-then-events model: every long-running command first emitted a `CommandPlan` (groups of `TaskPlan`s), then streamed `Task*` events referencing planned task IDs. The motivation was to let the CLI render the full task tree up front.

Since then, several structural issues have emerged:

- **The plan is a prediction; execution is non-deterministic.** From v0.3 (health checks, restart policies, `Backoff`), the daemon changes state autonomously. Retries, cancellation, concurrent commands, and a reconciler all cause a pre-computed plan to drift from runtime reality.
- **The plan pollutes daemon logic.** To keep the plan accurate, the daemon needs a dry-run planner sharing structure with the executor. Maintaining dual code paths (planning and executing) is a permanent cost paid solely for a UI visual feature.
- **Presentation leaks into the domain protocol.** `TaskGroup.header` is a purely visual concept. A "task row" was a UI abstraction whose link to a service was an untyped string, preventing non-CLI clients (IDE, TUI) from deriving typed service state from the stream.
- **Ambiguous naming.** Legacy event names were confusing (e.g., `TaskStarting` meant "began", whereas `TaskStarted` meant "completed successfully").
- **`Commit` is now a separate API operation.** Desired state is committed prior to execution, so the execution stream no longer needs to describe configuration changes as tasks. Remaining events describe strictly changes of service runtime state (`ServiceEvent`).
- **Terminology alignment.** The identifier tracking a streaming operation was previously named `execution_id`. To align with domain terminology, this field has been renamed to **`task_id`** across all RPCs and event messages (replacing `ExecutionAccepted` with `TaskAccepted`, `CancelExecution` with `CancelTask`, etc.).

## Decision

### 1. Remove the plan and UI abstractions from the protocol

Remove legacy `CommandPlan`, `TaskGroup`, `TaskPlan`, `TaskStarting`, `TaskStarted`, `TaskFailed`, `TaskSkipped`, `TaskCancelled` (legacy plan event), and `TaskError` from `proto/knot/v1/command.proto`. The daemon no longer constructs or serializes a plan.

### 2. The daemon emits typed facts (`ServiceEvent`), not predictions

All runtime state transitions are published as typed **`ServiceEvent`** instances within the `knot.v1.task` namespace. Every event carries the **`task_id`** (renamed from `execution_id`) of the invoking command (left empty if initiated autonomously by the daemon, e.g., health check, restart policy, future watcher).

### 3. Presentation moves to the client

The CLI constructs pending UI rows and groups from its local manifest (`[groups]`) and the `services` scope, then maps received `ServiceEvent` items onto `knot-terminal` calls (`Task::run_by_id`, `ok_by_id`, `fail_by_id`, `skip_by_id`). The terminal renderer remains unchanged. `TaskSequence` is not used for this flow because it assumes strict linearity, whereas the dependency DAG executes independent services in parallel.

### 4. Kept from ADR 0002 (with updated naming)

- Daemon-generated **`task_id`** (renamed from `execution_id`) and the session registry keyed by it, using `context.Context` for cancellation signaling.
- Unary `CancelTask` RPC (`CancelTaskRequest` / `CancelTaskResponse`) returning immediately.
- Rollback is reported directly on the **original** event stream (`Stopped` events for services being torn down, followed by the `TaskCancelled` terminal event), rather than requiring a secondary stream.

---

## Contract Invariants

1. **Stream Lifecycle:** The first event emitted on a command stream is `TaskAccepted`; the final event is a terminal command event (`UpCancelled`, `DownCancelled`, `TaskCancelled`, etc.). Nothing follows a terminal event.
2. **Service State Machine:** Events for a single service strictly follow the `ServiceStatus` state machine. Ordering between distinct services is guaranteed only to the extent implied by the dependency DAG.
3. **Bounded Execution:** Every execution reaches a terminal event. If a service fails to become ready, `Starting` is bounded by a timeout that produces `Failed`.
4. **Forward Compatibility:** A client logs and ignores events with unknown `oneof` variants or unknown service names without failing.

---

## Alternatives Considered

- **Keep the plan, fix its defects** (typed `subject`, `TaskProgress`, rename `TaskStarted`).  
  *Rejected:* Addresses naming and typing, but fails to fix the architectural flaw. The daemon would still need to maintain a dry-run predictor next to the executor, and plans would continue to drift.
- **Stream bare `ServiceStarted` / `ServiceStopped` with no task envelope.**  
  *Rejected:* Loses `task_id` correlation (concurrent commands cannot be distinguished), terminal status tracking, and the cancellation anchor established in ADR 0003.
- **Keep `TaskGroup` strictly for visual headers.**  
  *Rejected:* Group headers are derived from the project manifest, which the client already possesses. The daemon should not carry knowledge of UI display groupings.

---

## Consequences

- **Internal Event Bus:** The daemon requires an internal domain event bus. The runtime publishes `ServiceEvent`s, and command handlers subscribe by `task_id`. This same mechanism powers lifecycle event streams (e.g., a future `Watch` RPC).
- **No Predictor in Execution Path:** No dry-run planner is needed for streaming execution. A diff function `(committed, runtime) -> []Action` remains useful for `--dry-run`, but exists as a separate pure function outside the streaming protocol.
- **Terminal Rendering Isolation:** `knot-terminal` visual rendering remains unchanged; only the CLI event-mapping layer updates.
- **Dynamic Step Discovery:** The client cannot know the exact step breakdown in advance. UI rows are derived from the manifest and `services` scope; unexpected services dynamically register UI rows when their first `ServiceEvent` arrives.
- **Single Source for Terminal State:** Terminal status is derived directly from the event stream rather than computed via a parallel code path, preventing counter divergence.
- **Clean Migration (`proto/v1`):** `proto/v1` is unreleased, permitting breaking changes without deprecation cycles:
  1. Add event bus and `ServiceEvent` protobuf definition.
  2. Implement `TaskAccepted` event on command streams.
  3. Remove legacy `CommandPlan`, `TaskGroup`, `TaskPlan`, and legacy `Task*` types.
  4. Rename all references from `execution_id` / `CancelExecution` to `task_id` / `CancelTask`.
  5. Update client SDK mappings (`CommandHandle<E>` remains generic).