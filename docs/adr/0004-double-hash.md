# 0004 — Configuration hashes for drift detection

## Status

Accepted

## Context

The daemon must detect when its recorded workspace configuration no longer
matches the configuration currently present in the workspace. It must also be
able to localize drift to individual services so that synchronization can
determine the affected scope without comparing every service on every command.

A single hash for the complete configuration can detect that drift exists, but
cannot identify the changed services. The synchronization flow defined in ADR
0005 would then require a complete configuration comparison for every detected
mismatch.

## Decision

The daemon maintains two hashes for each workspace configuration:

- `runtime_hash` represents the configuration from which the daemon's current
  runtime state was derived.
- `reference_hash` represents the most recently registered workspace
  configuration.

If these values differ, the daemon marks the workspace configuration as
drifted. Equal values indicate that the runtime state was derived from the
latest registered configuration; they do not, by themselves, prove runtime
health.

The daemon calculates both workspace-level hashes and per-service hashes. A
workspace-level hash is derived from canonical workspace-level configuration
and the ordered set of `(service_id, service_hash)` entries. Entries are
ordered by stable `service_id` so source ordering cannot produce false drift.
The workspace-level hash provides a constant-time drift check during the
command handshake. Per-service hashes allow `Sync` to limit detailed
comparison and reconciliation to the services that changed.

Hashes are calculated from a canonical representation of configuration data.
The representation must be deterministic across process restarts and must
exclude volatile runtime values. Equivalent configurations must therefore
produce the same hash regardless of source ordering or serialization details.

## Alternatives considered

**Hash only the complete workspace configuration** — rejected because it
detects workspace-level drift but cannot identify the affected services. `Sync`
would need to compare the complete configuration after every mismatch.

**Compare complete configuration data on every command** — rejected because
it performs more work than necessary for the common no-drift case. Hash
comparison provides a cheap first-stage check, while detailed comparison is
deferred to synchronization.

**Hash individual services only** — rejected because the daemon would need to
inspect every service to determine whether the workspace has drifted. A
workspace-level hash provides the required aggregate check.

## Consequences

- Drift detection during the workspace handshake is a constant-time comparison
  of `runtime_hash` and `reference_hash`.
- Synchronization can identify and reconcile only the services whose hashes
  differ, reducing unnecessary work and producing more precise diagnostics.
- Canonicalization becomes part of the configuration contract. Changes to the
  canonical representation must be coordinated so they do not produce false
  drift reports.
- A hash mismatch signals configuration divergence, not a runtime failure;
  health checking remains a separate responsibility.
