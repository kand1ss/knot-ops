# 0005 — Workspace registration and lazy configuration synchronization

## Status

Accepted

## Context

The daemon must be able to identify workspaces, target commands at a specific
workspace, and detect when a workspace configuration has changed since it was
last observed. This is required both for auditing and for executing commands
against the correct configuration.

Performing a full configuration comparison before every command would make
routine operations unnecessarily expensive. Conversely, relying only on an
explicit synchronization command would allow commands to run against stale
daemon state without reporting it.

## Decision

`knot init` creates the workspace control directory and registers workspace
metadata. The metadata includes an immutable `workspace_id`, generated during
initialization. The daemon uses this identifier, rather than the workspace
path, as the stable identity for command routing and audit records.

Configuration freshness is managed in two stages:

1. **Handshake** — every workspace-scoped command performs a lightweight
   handshake before execution. The handshake verifies that the daemon is
   running and reachable through its IPC socket, confirms that the workspace
   is registered, and compares the current configuration hash with the hash
   recorded by the daemon. A mismatch marks the workspace as drifted; it does
   not perform a full comparison or modify daemon state.
2. **Sync** — an explicit synchronization operation performs the full drift
   analysis. It computes the changes between the current workspace
   configuration and the daemon's recorded configuration, then updates the
   daemon's workspace state and stored configuration hash.

Commands use the handshake result to select their behavior. Read-only commands
may report detected drift without synchronizing. Commands whose correctness
depends on current configuration, such as `knot up`, must run `Sync` before
performing work when the handshake detects drift.

This makes configuration change detection lazy: Knot observes changes when a
workspace-scoped command is invoked, rather than by continuously watching
workspace files.

## Alternatives considered

**Synchronize on every command** — rejected because calculating and applying
a full configuration diff for status and inspection commands adds avoidable
latency and work when no drift is present.

**Require users to run `knot sync` manually** — rejected because commands
that modify runtime state could otherwise proceed with stale configuration.
The command layer must be able to require synchronization when its correctness
depends on it.

**Continuously watch workspace configuration files** — rejected for now.
File watching introduces platform-specific behavior, lifecycle management, and
background resource use without being necessary for command-driven workflows.

## Consequences

- Every workspace has a stable identity that survives path changes and can be
  used consistently by the daemon, CLI, and audit records.
- All workspace-scoped commands incur the small cost of a reachability,
  registration, and hash-comparison check.
- Drift becomes visible to read-only commands without forcing a full sync.
- Commands that require current configuration have a deterministic recovery
  path: detect drift, synchronize, then execute.
