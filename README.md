# ⚡ knot - _Service_ orchestrator

> **One runtime. Every development service.**
>
> A local runtime for managing development services, regardless of how they are executed.

![CI](https://github.com/kand1ss/knot-ops/actions/workflows/ci.yml/badge.svg)
![CD](https://github.com/kand1ss/knot-ops/actions/workflows/cd.yml/badge.svg)
[![codecov](https://codecov.io/github/kand1ss/knot-ops/graph/badge.svg?token=KHVGOUH2O4)](https://codecov.io/github/kand1ss/knot-ops)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)]()

> 🚧 **Work in Progress** — knot is under active development. APIs and configuration formats may change.
> 
> *Roadmap you can find at [knot-ops/roadmap](https://github.com/kand1ss/knot-ops/blob/main/docs/roadmap.md).*

---

## Why Knot?

Modern applications are built from **services**, not processes.

A service might be:

- a PostgreSQL database running in Docker
- a backend started with `cargo run`
- a frontend running through `npm run dev`
- a Python worker
- a Redis container

Most existing tools focus on **how** services are started.

Knot focuses on **how they live**.

It provides a single runtime responsible for:

- starting services
- tracking their lifecycle
- resolving dependencies
- monitoring health
- restarting failures
- exposing logs and runtime state

The execution backend is simply an implementation detail.

---

## Philosophy

> Knot is **not** trying to replace Docker Compose or Kubernetes.

Docker Compose is excellent at orchestrating containers.

Knot solves a different problem.

It provides a runtime that manages the lifecycle of development services regardless of how they are executed.

Containers are one execution backend.

Processes are another.

The runtime remains the same.

---

## Runtime Model

Knot manages **Services**.

Every service has:

- an execution backend
- dependencies
- health checks
- restart policy
- runtime state
- logs

Whether the service runs as:

- a native process
- a Docker container
- a future execution backend

doesn't change how the runtime interacts with it.

This unified model allows every service to participate in the same dependency graph and lifecycle regardless of how it is executed.

---

## Example

```toml
[services.postgres]
type = "docker"
image = "postgres:17"

[services.backend]
type = "process"
cmd = "cargo run"
depends_on = ["postgres"]

[services.frontend]
type = "process"
cmd = "npm run dev"
depends_on = ["backend"]
```

```console
$ knot up

✓ postgres (tag: latest) - healthy
✓ backend (pid: 5432) - running
✓ frontend (pid: 4321) - running

>>> All services are ready.
```

The backend doesn't start when PostgreSQL **starts**.

It starts when PostgreSQL is actually **ready**.

---

## Core Principles

### Runtime-first

Knot is built around a long-running daemon.

The CLI is only a client.

The daemon owns:

- service lifecycle
- runtime state
- dependency graph
- event stream
- health monitoring

This architecture enables persistent state, crash recovery, multiple clients and future IDE integrations.

---

### Services are first-class objects

A service is the fundamental abstraction.

Processes and containers are simply different execution strategies.

The runtime treats them identically.

---

### Health-aware orchestration

Dependencies are resolved using runtime readiness.

Instead of:

```
Start PostgreSQL

↓

Start Backend
```

Knot executes:

```
Start PostgreSQL

↓

Wait until healthy

↓

Start Backend

↓

Wait until healthy

↓

Start Frontend
```

The runtime always knows which services are:

- Starting
- Running
- Waiting
- Unhealthy
- Stopped

---

### Declarative configuration

Projects are described using a simple TOML configuration.

Developers declare **what** services exist.

Knot determines **when** and **how** they should run.

---

## Features

### Unified Runtime

Manage every development service through a single runtime.

- Native processes
- Docker containers
- Future execution backends

---

### Dependency Graph

Services start in dependency order.

Dependencies become active only after health checks succeed.

---

### Health Engine

Built-in health checks:

- HTTP
- TCP
- Command

Future extensions may support custom health providers.

---

### Runtime State

The daemon continuously tracks:

- lifecycle state
- exit codes
- health status
- uptime
- restart history

The runtime state is persisted and survives daemon restarts.

---

### Logs

Each service exposes:

- live logs
- buffered history
- structured events

through a unified interface.

---

### Execution Backends

Execution is pluggable.

Today:

- Process Runner

Planned:

- Docker Runner
- Podman Runner
- Remote Runner

---

## Current Status

Knot is currently in active development.

The architecture is intentionally designed around long-term runtime concepts rather than CLI features.

Expect breaking changes until the first stable release.

---

## License

Licensed under the MIT License.
See [LICENSE](LICENSE).