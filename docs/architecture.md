# Architecture

## Overview
Knot follows a daemon/client architecture where a background
process manages services and CLI communicates with it via IPC.


## Crates
- **knot-core** — domain types, errors.
- **knot-transport** — IPC protocol, message types
- **knot-daemon** — background process. Manages services, checks healths, 
- **knot-cli** — main client. Speaks with daemon via IPC protocol.

## Transport
Messages between CLI and Daemon use a length-prefixed
JSON protocol over Unix sockets (Named Pipes on Windows).

See `knot-transport/src/` for implementation details.