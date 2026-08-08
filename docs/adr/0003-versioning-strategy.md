# 0003 - Protocol and Binary Versioning Strategy

## Status

Accepted

## Context

Knot exposes a gRPC API that is consumed by the CLI, TUI, and external SDKs. It is also distributed as compiled binaries, including the daemon and command-line clients.

The protocol and the binaries have different compatibility requirements. A binary release can contain implementation changes that do not affect the wire contract, while a protocol-breaking change may require old and new clients to coexist during a migration. Using one version number for both would make those changes unnecessarily coupled.

## Decision

Version the protocol and released binaries independently.

### 1. Protocol Versioning

Protocol versions are part of the Protocol Buffers package namespace and directory path. The current API is defined under `proto/knot/v1/` with package `knot.v1`.

* A new protocol major version, such as `v2`, is introduced only for breaking wire-contract changes. Examples include removing or renaming RPCs, changing field types, or changing field numbers.
* Changes within a protocol major version must be backward compatible. Allowed changes include adding optional fields, new RPCs, and new enum values.
* Removed fields must be marked `reserved`; field numbers must never be reused.
* During a migration, the daemon may serve multiple protocol versions concurrently so clients can upgrade on their own release schedule.

### 2. Binary Release Versioning

Released binaries follow Semantic Versioning, for example `v0.1.0`.

* A binary version change does not imply a protocol namespace change.
* A binary release that introduces a new protocol version must state the supported protocol versions in its release notes and compatibility documentation.
* Binaries may add support for a new protocol version before it becomes the default, allowing staged upgrades.

## Alternatives Considered

### Single Global Version

Rejected. A release version cannot express protocol compatibility precisely: implementation-only releases would appear to change the wire contract, while a protocol migration would be difficult to identify and support in parallel.

### Binary Versioning Only

Rejected. Without explicit versions in the Protocol Buffers namespace, serving compatible old and new APIs side by side during a migration is ambiguous and difficult to maintain.

## Consequences

* Protocol compatibility is explicit in the API namespace and independent of binary release cadence.
* Clients can remain on a supported protocol version while binaries are upgraded incrementally.
* Breaking protocol changes require a new namespace and a documented compatibility window, making migrations deliberate and predictable.
