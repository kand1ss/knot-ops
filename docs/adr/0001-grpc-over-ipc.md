# 0001 — gRPC over IPC as the daemon transport

## Status

Accepted

## Context

The knot daemon needs to communicate with multiple clients (CLI, TUI, SDKs in
Rust/Go/Python, and future platform components) over a local IPC channel
(Unix domain sockets on Linux/macOS, Named Pipes on Windows).

Three options were considered:

1. **Custom binary protocol** — length-prefixed framing with a hand-rolled
   request/response correlation layer (the original `knot-transport`
   implementation).
2. **HTTP + JSON over UDS** — the approach used by Docker
   (`/var/run/docker.sock`).
3. **gRPC over IPC** — protobuf-defined contract, generated clients/servers,
   transported over UDS/Named Pipes instead of TCP.

The custom protocol was already implemented and worked (see the
`knot-transport` performance audit — ~175µs per-request latency,
~47 MiB/s peak throughput). However, two requirements emerged that the
custom protocol does not solve well:

- **Server-initiated streaming** — the `Up`/`Down` commands need to push a
  `Plan` followed by a live sequence of task events to the CLI as they
  happen. Implementing this on the custom protocol means building broadcast
  channels, subscriber lifecycle management, and stream framing by hand.
- **Multi-language clients** — knot is designed as an extensible platform.
  Components and SDKs may be written in Rust, Go, Python, or other
  languages. A hand-rolled protocol means re-implementing framing and
  correlation in every language.

## Decision

Replace the custom transport with **gRPC over IPC**:

- Contracts are defined in `.proto` files under `proto/knot/v1/`.
- Transport is Unix Domain Sockets (Unix) / Named Pipes (Windows), not TCP —
  avoiding network stack overhead and port management for a purely local
  daemon.
- Server-streaming RPCs (`Up`, `Down`, `Logs`) handle live
  events natively through gRPC streaming.
- Each language gets generated client/server code from the same `.proto`
  source of truth.

On Rust, this required a custom `tower::Service<Uri>` connector
(`IpcConnector`) and a custom `Stream`-based listener (`IpcIncoming`) to
bridge `interprocess`/`tokio::net::UnixStream` with `tonic`/`hyper`. On Go,
`net.Listen("unix", ...)` and `grpc.Dial` work with UDS natively — no
adapter layer needed.

### IPC endpoint placement and access control

The daemon exposes one host-wide endpoint, not one endpoint per workspace or
operating-system user. Workspace identity is carried in the protocol and is
not encoded in the socket path. Workspace control data such as
`.knot/metadata.json` must not contain a live IPC endpoint.

On Unix, the endpoint is stored at `/run/knot/knot.sock`. On systems where
`/var/run` is a compatibility symlink to `/run`, this is also reachable as
`/var/run/knot/knot.sock`. The daemon service creates `/run/knot` with owner
`root:knot` and mode `0750`; the socket is owned by `root:knot` with mode
`0660`. The `knot` group is the access-control boundary: only root and its
members may connect to the daemon.

The runtime directory must be created by the service manager or during
privileged installation, not by an unprivileged CLI invocation. A listener
must be created inside that protected directory before it becomes available to
clients. Stale endpoints may be removed only after verifying that the path is
a socket owned by the service account; any other existing file is an error.

Membership in the `knot` group grants the ability to issue every operation
available through the local daemon API. Administrators must therefore manage
this group with the same care as other privileged local-service groups. If
future requirements need different permissions for different users, the daemon
must authenticate the connecting process and enforce authorization in the
protocol; socket permissions alone are not sufficient.

On Windows, the endpoint is a single Named Pipe, for example
`\\.\pipe\knot`. Its security descriptor must grant access to the designated
local Knot users or group and to the local system account when required by the
service model. The pipe name alone is not an access-control mechanism.

## Alternatives considered

**HTTP + JSON over UDS (Docker-style)** — simpler, human-debuggable with
`curl --unix-socket`, but server-side streaming is awkward over plain HTTP/1.1
request/response, and there's no code generation — every client
re-implements the JSON shapes by hand.

**Keep the custom protocol, add JSON framing for language-agnosticism** —
this was the fallback if gRPC proved too heavy. Rejected because it still
requires hand-building the event broadcast/subscription mechanism that gRPC
streaming provides for free.

**Store the Unix socket in `.knot/`** — rejected because the daemon is
host-wide, not workspace-scoped. Workspace directories may also be on shared
filesystems or writable by users other than the daemon owner, allowing endpoint
replacement and client impersonation.

**Allow every local user to connect to the system socket** — rejected because
the daemon can manage registered workspaces and their runtime state. A
world-writable socket would make every local account a daemon client without
an authorization boundary.

## Consequences

- `.proto` files become the platform's public contract. Versioning
  discipline (see ADR 0003) is required from day one.
- Adds `protoc`/`protox` as a build-time dependency for code generation.
- The daemon endpoint is shared by authorized local users. The `knot` group
  is security-sensitive because its members receive full daemon access.
- Existing clients that derive the endpoint from a workspace directory must be
  migrated to resolve `/run/knot/knot.sock` instead.
- Migration was scoped narrowly because the transport was already hidden
  behind the `KnotClient` trait — only the trait's implementation changed,
  not its consumers (CLI, TUI).
