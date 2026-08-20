# 0003 — CommandPlan and unified cancellation

## Status

Accepted

## Context

Commands like `Up` and `Down` execute a sequence of tasks (start/stop
services, run health checks, etc.) and need to report progress to the CLI
in real time so it can render a live, collapsing task tree (see the
`knot up` terminal rendering design).

Two requirements drove this design:

- The CLI needs to know the **full shape of the work** (which tasks will
  run, grouped how) before execution starts, so it can render all tasks
  up front (dim/pending) rather than discovering them one at a time.
- The user needs to be able to **cancel** a running command (Ctrl+C) and
  see the daemon roll back whatever was already done — not just have the
  connection silently drop.

Initially, `Plan`/`session_id`/`CancelUp`/`CancelDown` were modeled as
`Up`-specific concepts. As the design solidified, it became clear that
*any* command which executes a sequence of tasks (`Up`, `Down`, and future
commands like `Restart` or `Build`) needs the same plan-then-events shape
and the same cancellation mechanism.

## Decision

Extract a shared **command** layer (`proto/knot/v1/command.proto`)
used by any command with this shape:

- `CommandPlan` — sent as the first event of any such command's stream.
  Contains a daemon-generated `command_id` and `TaskGroup`s (each with an
  optional header for visual grouping and a list of `TaskPlan`s).
- Common task-level events — `TaskStarting`, `TaskFailed`, `TaskSkipped`,
  `TaskCancelled` — shared across `UpEvent`, `DownEvent`, and future
  `*Event` types. Command-specific events (e.g. `TaskStarted` carrying
  `ServiceStarted` for `Up`) remain in their command's own `.proto` file.
- A single `CancelCommand(execution_id, reason)` RPC, instead of
  `CancelUp`/`CancelDown`/etc per command.

**Cancellation does not use a separate response stream.** 
`CancelCommand` is a lightweight unary RPC that returns immediately
(`cancelled: bool` — whether an active execution was found). The actual
rollback process (`ServiceStopped` for services being torn down,
`TaskCancelled` for the in-flight task, and a final `UpCancelled` /
`DownCancelled`) continues to flow through the **original** command's
stream that the CLI is already listening to. This avoids merging two event
streams in the UI and keeps a single source of truth for execution state.

On the daemon side, each execution is tracked in a session registry keyed
by `command_id`, using `context.Context` (Go) for cancellation signaling
— `ctx.Done()` can be observed from multiple points (the main task loop,
in-flight health checks) without the "channel consumed once" problem a raw
channel would have.

## Alternatives considered

**Per-command cancellation (`CancelUp`, `CancelDown`, ...)** — rejected
because it does not scale to new commands and fragments the session
registry by command type for no benefit; `command_id` is unique
regardless of which command produced it.

**Streaming response for `CancelCommand`** — considered, but rejected:
the rollback events are already expressible as existing event types
(`ServiceStopped`, `TaskCancelled`) in the original stream. A second stream
would require the CLI to interleave two sources of events for the same
logical operation, introducing potential ordering ambiguity.

**Binding cancellation to the gRPC connection/stream context directly**
(relying on `context.Done()` when the client disconnects) — insufficient
on its own: a clean Ctrl+C should trigger *graceful* rollback (stop started
services, report `UpCancelled`), not just an abrupt stream termination. An
explicit `command_id` and RPC give the daemon a clear signal to begin
rollback rather than just observing a dropped connection.

## Consequences

- `Plan` was renamed to `CommandPlan` to reflect that this is a general mechanism, not `Up`-specific.
- `TaskGroup` (with optional `header`) replaced an earlier `oneof`-based
  `PlanItem` (task | separator) design — simpler for renderers, at the cost
  of not supporting separators without an associated task group (acceptable
  given the actual use cases).
- New commands that execute task sequences (e.g. a future `Restart`) get
  cancellation for free by emitting `CommandPlan` and handling
  `CancelCommand` — no new RPC needed.
- The CLI's `CommandHandle<E>` wraps a stream of `E` (`UpEvent` /
  `DownEvent`) plus a shared `cancel()` method backed by
  `CancelCommand` — `UpHandle`/`DownHandle` become thin aliases.