# knot-ops | 🚧 Work in progress |

> A fast local service orchestrator for developers — built in Rust.

[![Crates.io](https://img.shields.io/crates/v/knot-ops.svg)](https://crates.io/crates/knot-ops)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)]()

Knot manages your local development services — starts them in the right order, watches their health, and restarts them when they fail. One binary, no dependencies, no Docker required.

```
$ knot up

  ⠸ postgres    starting...
  ✓ postgres    running  (3.2s)

  ⏸ backend     waiting  (depends on: postgres)
  ⠸ backend     starting...
  ✓ backend     running  (11.8s)

  ⠸ frontend    starting...
  ✓ frontend    running  (4.1s)

  ✓ all services running
```

---

## Why knot?

Most local dev setups rely on a patchwork of shell scripts, `docker-compose`, and `Procfile` runners — none of which know when your services are actually *ready*.

Knot is different:

- **Dependency-aware startup** — services start in the correct order and only proceed when their dependencies pass health checks, not just when the process spawns.
- **Real health checks** — TCP, HTTP, or custom command. Backend won't start until `pg_isready` succeeds.
- **Crash recovery** — the engine persists state to disk. If it crashes, it reattaches to your running processes and picks up where it left off.
- **Single binary** — one `curl` install, no Python, no Node, no Docker daemon required.
- **Extensible** — lifecycle hooks in bash, PowerShell, or Python for notifications, migrations, and custom logic.

---

## Quick Start

**1. Initialise a project:**
```bash
cd my-project
knot init
```

**2. Define your services in `Knot.toml`:**
```toml
[project]
name = "my-app"

[services.postgres]
image = "postgres:16"
port  = 5432
env   = { POSTGRES_PASSWORD = "secret" }

  [services.postgres.healthcheck]
  cmd     = "pg_isready -U postgres"
  timeout = "5s"
  retries = 10

[services.backend]
cmd        = "cargo run"
dir        = "./backend"
port       = 8080
depends_on = ["postgres"]
restart    = "on-failure"

  [services.backend.healthcheck]
  http    = "http://localhost:8080/health"
  timeout = "3s"

[services.frontend]
cmd        = "npm run dev"
dir        = "./frontend"
depends_on = ["backend"]
```

**3. Start everything:**
```bash
knot up
```

---

## Commands

### Core
```bash
knot init              # Initialise knot in the current directory
knot up                # Start the engine and all services
knot down              # Stop all services and the engine
knot restart           # Restart all services
knot status            # Show current status of all services
```

### Services and groups
```bash
knot up   -s postgres          # Start a single service
knot down -s backend           # Stop a single service
knot up   -g infra             # Start a named group of services
knot down -g app               # Stop a named group of services
knot restart -s backend        # Restart a single service
```

### Logs
```bash
knot logs backend              # Print recent log lines
knot logs backend -f           # Stream live log output
knot logs --all                # Print logs from all services
knot logs backend --count 50   # Print last 50 lines
```

### Engine (daemon)
```bash
knot daemon start              # Start the engine without starting services
knot daemon stop               # Stop the engine (services keep running)
knot daemon status             # Show engine PID, uptime, and settings
```

### Config
```bash
knot check                     # Validate Knot.toml and project setup

knot set -s backend restart="always"
knot set -d log_level="debug"

knot remove -s worker
```

### Dashboard
```bash
knot tui                       # Open the live terminal dashboard
```

---

## Configuration Reference

### `[services.<name>]`

```toml
[services.backend]
# Process type (one of cmd or image is required)
cmd        = "cargo run"       # Shell command to run
image      = "postgres:16"     # Docker image (requires type = "docker")
type       = "process"         # "process" (default) or "docker"

# Location
dir        = "./backend"       # Working directory (default: project root)
port       = 8080              # Primary port (used for default health check)

# Dependencies
depends_on = ["postgres"]      # Start after these services are healthy

# Environment
env        = { KEY = "value" } # Inline environment variables
env_file   = ".env"            # Load variables from a file

# Restart policy
restart    = "never"           # "never" | "always" | "on-failure"

# File watching (restart on change)
watch      = "src/**/*.rs"
```

### `[services.<name>.restart]` — detailed form

```toml
[services.backend.restart]
policy      = "on-failure"
max_retries = 5
delay       = "1s"
max_delay   = "30s"
multiplier  = 2.0
```

Backoff sequence with `delay = "1s"` and `multiplier = 2.0`:
`1s → 2s → 4s → 8s → 16s → 30s (capped)`

### `[services.<name>.healthcheck]`

```toml
[services.backend.healthcheck]
# One of:
http    = "http://localhost:8080/health"   # HTTP GET — expects 2xx
tcp     = 8080                             # TCP connection check
cmd     = "pg_isready -U postgres"         # Shell command — expects exit 0

# Options (all optional)
interval = "5s"    # How often to check (default: 5s)
timeout  = "10s"   # Per-check timeout   (default: 10s)
retries  = 3       # Failures before Unhealthy (default: 3)
```

### `[services.<name>.watch]` — detailed form

```toml
[services.backend.watch]
patterns = ["src/**/*.py", "config/*.yaml"]
debounce = "300ms"
ignore   = ["**/__pycache__", "**/*.pyc"]
```

### `[groups]`

```toml
[groups]
infra = ["postgres", "redis"]
app   = ["backend", "frontend"]
```

### `[daemon]`

```toml
[daemon]
log_level         = "info"      # trace | debug | info | warn | error
log_format        = "pretty"    # pretty | json | compact
startup_timeout   = "60s"
shutdown_timeout  = "30s"
log_buffer_lines  = 1000
log_max_size      = "50MB"
transport_timeout = "5s"
```

---

## Python SDK

For projects that need dynamic configuration or advanced hook logic, knot supports Python.

**Initialise with Python support:**
```bash
knot init --python / -p
```

This creates a `.knot/venv/` with `knot-sdk` pre-installed.

**`Knotfile.py` — configuration as code:**
```python
from knot import service, docker, group
import os

IS_CI = os.getenv("CI") == "true"

postgres = docker("postgres:16",
    port=5432,
    env={"POSTGRES_PASSWORD": os.getenv("DB_PASS", "secret")},
)

backend = service("cargo run",
    dir="./backend",
    port=8080,
    depends_on=[postgres],
    restart="always" if IS_CI else "on-failure",
)

group("infra", [postgres])
group("app",   [backend])
```

**Hooks:**
```python
# hooks/lifecycle.py
from knot import hook, HookEvent, HookContext

@hook(HookEvent.BEFORE_START, service="backend")
def run_migrations(ctx: HookContext):
    import subprocess
    subprocess.run(["python", "manage.py", "migrate"], check=True)

@hook(HookEvent.ON_FAILURE)
def notify_slack(ctx: HookContext):
    import requests
    requests.post(SLACK_URL, json={
        "text": f"Service `{ctx.service}` failed (exit {ctx.exit_code})"
    })
```

```toml
# Knot.toml
[hooks]
files = ["hooks/lifecycle.py"]
```

---

## How It Works

Knot runs as a background engine (daemon) and a CLI that communicates with it over a Unix socket (Named Pipe on Windows).
The engine persists its runtime state to `.knot/state.db`. If it crashes or is restarted, it reads the state on boot, checks which processes are still alive, and reattaches to them — without restarting your services.

---

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.
