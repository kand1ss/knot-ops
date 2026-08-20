# 0007 — Defer system-managed daemon mode

## Status
Accepted (supersedes daemon-placement portions of 0001, obsoletes 0006)

## Context
ADR-0001 established a privileged system-wide endpoint (/run/knot/knot.sock)
alongside implicit expectation of a user-session fallback. ADR-0006 formalized
this as two supervision modes with explicit priority resolution and
shadow-daemon detection between them.

Analysis of actual v0.1–v0.4 roadmap requirements found no concrete consumer
for multi-user or logout-surviving daemon behavior. The dual-mode model's
cost — priority resolution, shadow detection, migration UX, per-platform
unit installation (systemd/launchd/SCM), and duplicated logic burden on
planned Go/Python SDKs — is not justified by any near-term requirement.

## Decision
Collapse to a single supervision mode for v0.1 onward: user-session daemon
at $XDG_RUNTIME_DIR/knot/knot.sock, spawned and supervised directly by the
CLI via flock-based advisory locking. The system-managed mode described in
ADR-0006 is deferred, not implemented, until a concrete driver emerges
(see roadmap "Deferred" section).

## Alternatives considered

**System-managed only (invert: drop UserSession, keep SystemManaged)** —
rejected. Requires privileged unit installation before first `knot up`,
contradicting the zero-friction onboarding promised in README. Makes the
tool unusable in containers/CI where systemd is not PID 1 — a primary
target environment for dev orchestration tooling. Ad-hoc spawn logic would
not actually disappear; it would migrate to test-only code paths or
undocumented fallbacks, which is worse than an explicit single mode.

## Consequences
- ADR-0001's socket placement section (/run/knot, root:knot ownership) is
  no longer current guidance; superseded by this ADR for v0.1–v0.4.
- DaemonSupervisionMode enum, shadow-daemon detection, and `knot daemon
  migrate` are removed from the implementation; this ADR is the record of
  why, preventing reintroduction without a documented trigger.
- Multi-user shared daemon and headless persistence become explicitly
  unsupported until re-scoped — must be communicated in docs, not silently
  absent.