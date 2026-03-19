# Roadmap

> Each version delivers a working tool. No version leaves the project in a broken state.

---

## v0.1 — Foundation
**Goal: Base features — start and stop services**

### Daemon (Engine)
- Event Bus (coordination component)
- TOML Parsing via Config Collector
- DAG Engine — builds service dependency graph
- Supervisor — manages services
- Runtime State Store (in-memory)
- Service statuses: `Stopped`, `Starting`, `Running`, `Failed`

### Transport
- IPC (Unix Socket / Named Pipes)

### CLI
- `knot init` — initialize project
- `knot up` — start engine and services
- `knot down` — stop engine and services

### Config — `[services.<service>]`
```toml
[services.backend]
cmd        = "cargo run"
dir        = "./backend"
env        = { KEY = "value" }
env_file   = ".env"
depends_on = ["postgres"]
```

---

## v0.2 — Observability
**Goal: Improving service observability**

### Daemon
- Health Checker: `Command`, `Tcp`, `Http`, `ProcessAlive` strategies
- Extended service statuses: `Waiting`, `Starting`, `Running`, `Degraded`, `Restarting`, `Stopping`, `Stopped`, `Failed`
- Log Aggregator

### CLI
- `knot status`
- `knot logs <service>` / `knot logs --all`
- `knot logs --count <n>`
- `knot logs --follow` / `-f`

### Config — `[services.<service>]`
```toml
[services.backend]
healthcheck = 

[services.backend.healthcheck]
url     = "http://localhost:8080/health"
port    = 8080
cmd     = "pg_isready"
timeout = "5s"
```

---

## v0.3 — Reliability
**Goal: Improving tool reliability, implementing persistent state store**

### Daemon
- Runtime State Store (SQLite database)
- Crash Recovery — restore state after daemon restart
- Restart Policy: `Never`, `Always`, `OnFailure`, `Backoff`

### CLI
- `knot restart`
Auto-recovery after daemon crash

### Config — `[services.<service>]`
```toml
[services.backend]
restart = "on-failure"

[services.backend.restart]
policy      = "on-failure"
max_retries = 5
delay       = "2s"
max_delay   = "30s"
multiplier  = 2.0
```

---

## v0.4 — Groups & Engine Extensions
**Goal: Launching individual services and service groups. New daemon CLI features**

### CLI
- `knot up --service` / `-s <service>`
- `knot up --group` / `-g <group>`
- `knot down --service` / `-s <service>`
- `knot down --group` / `-g <group>`
- `knot daemon start`
- `knot daemon stop`
- `knot daemon status`

### Config
```toml
[groups]
infra = ["postgres", "redis"]
app   = ["backend", "frontend"]
```

---

## v0.5 — Config Extensions
**Goal: Config management through CLI with validation. Daemon config. Global services configuration**

### v0.5a — Daemon & Global Configuration

#### Config
```toml
[daemon]
log_level         = "info"
log_format        = "pretty"
state_db          = ".knot/state.db"
log_buffer_lines  = 1000
log_max_size      = "50MB"
transport_timeout = "5s"

[services]
env        = { KEY = "value" }
env_file   = ".env"
restart    = "never"
healthcheck = { ... }
```

### v0.5b — Config CLI

#### CLI
```bash
knot add --service / -s  
knot add --group  / -g  []
knot remove --service / -s 
knot remove --group   / -g 
knot set --service / -s   
knot set --group   / -g    
knot set --daemon  / -d  
```

---

## v0.6 — TUI Dashboard
**Goal: Implementation of LIVE dashboard**

### TUI
- Table of services: Name, Status, PID, Uptime, Restarts
- Live logs panel
- Navigation between services
- Hotkeys

### CLI
- `knot tui`

---

## v0.7 — Extensibility
**Goal: Extensibility through lifecycle hooks and Python SDK**

### v0.7a — Lifecycle Hooks

#### Daemon
- Hook Dispatcher

#### Hook Runner
- Hook Executor
- Hook Collector
- Platform support:
  - Windows: `.bat`, `.ps1`, `.cmd`
  - Linux: `.sh`, `.zsh`, `.bash`

---

### v0.7b — Python SDK

#### Python Lib
- `service()`
- `group()`
- `@hook`
- `HookContext`, `HookEvent`

#### Daemon
- Python Bridge (PyO3)
- Python Hook Collector
- Python Config Collector
- Python Hooks Registry
- Python Config Registry

#### Hook Runner
- `Python: .py`

#### Config
- Support for `.py` config via `Knotfile.py`

---

## v0.8 — Docker Integration
**Goal: Implementing Docker integration**

### Daemon
- `DockerRunner` — manages container lifecycle

### Config
```toml
[services.postgres]
type  = "docker"
image = "postgres:16"
port  = 5432
```

---

## v0.9 — Polish & DX
**Goal: Polishing and adding developer experience features**

### CLI
- `knot check` — checks initialization, validates config

### Daemon
- File Watcher — watches project filesystem, restarts service on file change

### Config
```toml
[services.backend]
watch = "src/**/*.py"

# or multiple patterns
watch = ["src/**/*.py", "config/*.yaml"]

# or with options
[services.backend.watch]
patterns = ["src/**/*.py"]
debounce = "300ms"
ignore   = ["**/__pycache__", "**/*.pyc"]

# $ENV variable support
[services.backend]
cmd = "cargo run --bin $APP_NAME"
```

## v0.10 - Components & Pipeline features
**Goal: Adding prepared components and the ability to create your own**

### Config
**Component using:**
```toml
[services.postgres]
use = "knot/postgres" # name of component

[services.backend]
cmd        = "cargo run"
depends_on = ["postgres"] # exported variables will be injected in service
```

**Component declaration:**
```toml
[components.postgres]
description = "PostgreSQL database"
version     = "16"
type  = "docker"
image = "postgres:16"
port  = 5432
restart = "always"

  [components.postgres.env]
  POSTGRES_PASSWORD = "{{ required | env: POSTGRES_PASSWORD }}"
  POSTGRES_DB       = "{{ project.name }}"
  POSTGRES_USER     = "postgres"

  [components.postgres.healthcheck]
  cmd     = "pg_isready -U {{ env.POSTGRES_USER }}"
  timeout = "5s"
  retries = 10

  # variables which component will export
  [components.postgres.exports]
  DATABASE_URL = "postgres://{{ env.POSTGRES_USER }}:{{ env.POSTGRES_PASSWORD }}@localhost:{{ port }}/{{ env.POSTGRES_DB }}"
```

### Python SDK
**Component using:**
```python
from knot import service, group
from knot.components import use

# using prepared component
postgres = use("knot/postgres",
    password = "secret",
    db       = "myapp",
)

# using own component
redis = use("my_component",
    port = 6380,
)

# backend automatically gets DATABASE_URL from postgres
backend = service("cargo run",
    dir        = "./backend",
    port       = 8080,
    depends_on = [postgres, redis],
    # DATABASE_URL will be injected automatically
)

group("infra", [postgres, redis])
group("app",   [backend])
```

**Component declaration:**
```python
from knot import component, field, export, healthcheck

@component(
    name    = "knot/postgres",
    version = "16",
    description = "PostgreSQL database",
)
class PostgresComponent:
    # fields with defaults
    image:    str = field(default="postgres:16")
    port:     int = field(default=5432)
    user:     str = field(default="postgres")
    password: str = field(required=True)
    db:       str = field(default=lambda ctx: ctx.project.name)

    def configure(self):
        return {
            "type":  "docker",
            "image": self.image,
            "port":  self.port,
            "env": {
                "POSTGRES_USER":     self.user,
                "POSTGRES_PASSWORD": self.password,
                "POSTGRES_DB":       self.db,
            },
            "healthcheck": healthcheck.command(
                cmd     = f"pg_isready -U {self.user}",
                timeout = "5s",
                retries = 10,
            ),
            "restart": "always",
        }

    @export
    def DATABASE_URL(self) -> str:
        return (
            f"postgres://{self.user}:{self.password}"
            f"@localhost:{self.port}/{self.db}"
        )

    @export
    def POSTGRES_HOST(self) -> str:
        return "localhost"

    @export
    def POSTGRES_PORT(self) -> int:
        return self.port
```