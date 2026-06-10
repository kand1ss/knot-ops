# knot-terminal

A beautiful, concurrent terminal progress reporting and task orchestration library for Rust, built on top of `indicatif` and `console`.

It supports both interactive terminals with fancy live progress bars (using dynamic layout management) and non-interactive plain streams (such as CI systems or piped files) automatically.

## Key Features

- **Adaptive Rendering**: Automatically switches between **Interactive** (dynamic layouts, animated spinners, custom arrows/prefixes) and **Plain** (stdout line-based, ANSI stripped, CI-friendly) modes.
- **Hierarchy & Layout Management**: Safely anchors progress bars and groups them sequentially. Active subtasks appear cleanly under parent titles and disappear upon success to keep terminal outputs neat and professional.
- **Task Orchestration**: Model execution steps using `Step`, `Task`, and `TaskSequence` to represent individual actions, complex multi-stage tasks, or strict step-by-step sequences.
- **Preserved Status & Diagnostics**: Supports embedding multi-line context-rich `ErrorReport`s directly into the console stream under a failing task.
- **Beautiful presets**: Comes with modern/unicode styles default to high-end terminal apps, and ASCII/plain fallback configurations.

## Architecture & Concepts

1. **TaskEngine**: The central configuration entry point. Detects if stdout is a TTY/CI, sets up the layout anchor system, holds custom styling (`TaskStyle`), and manages the active rendering mode.
2. **Step**: Represents a standalone, indivisible operation (e.g. running a quick command or a generic background step).
3. **Task**: Represents a complex operation made up of multiple subtasks called **Stages**. It orchestrates their overall completion and displays a root progress bar.
4. **TaskSequence**: A specialized state machine runner ensuring that stages are completed strictly one after another (e.g., pulling images -> starting containers -> healthcheck).

---

## Usage Examples

Add the dependency to your `Cargo.toml`:
```toml
[dependencies]
knot-terminal = { path = "path/to/src/terminal" }
```

### 1. Basic Standalone Step
```rust
use knot_terminal::TaskEngine;
use std::thread;
use std::time::Duration;

fn main() {
    let engine = TaskEngine::new();
    
    // Create and auto-start a step
    let mut step = engine.step("Compile package", true);
    
    thread::sleep(Duration::from_millis(500));
    
    // Mark completed
    step.ok("compiled 12 modules");
}
```

### 2. Multi-stage Task with Grouping
```rust
use knot_terminal::{TaskEngine, ErrorReport};

fn main() {
    let engine = TaskEngine::new();
    
    let mut task = engine.task("Database migration")
        .with_group(Some("Pre-checks"))
        .with_stage("check_schema", "Validate current schema version", true)
        .with_group(Some("Migration Stages"))
        .with_stage("apply_migrations", "Apply SQL migration scripts", false)
        .start(true); // auto_indent = true
        
    // Work on stage 1
    std::thread::sleep(std::time::Duration::from_millis(300));
    task.ok_by_id("check_schema", "Schema is clean");
    
    // Start and work on stage 2
    task.run_by_id("apply_migrations", Some("Executing step 3/5"));
    std::thread::sleep(std::time::Duration::from_millis(500));
    
    // Suppose it fails
    let error = ErrorReport::new("Failed to apply migration V3__add_users_index.sql")
        .with_context("SQL Error: relation \"users\" already exists")
        .with_solution("Run `cargo sqlx db reset` to restore database consistency.");
        
    task.fail_by_id("apply_migrations", error);
}
```

### 3. Sequential Executions
```rust
use knot_terminal::TaskEngine;

fn main() {
    let engine = TaskEngine::new();
    
    let mut seq = engine.sequence("Deploy application")
        .with_stage("Building binaries")
        .with_stage("Uploading assets")
        .with_stage("Restarting server")
        .start(true);
        
    // First stage ("Building binaries") is automatically started on start().
    std::thread::sleep(std::time::Duration::from_millis(400));
    seq.ok("built release binary"); // Automatically starts "Uploading assets"
    
    std::thread::sleep(std::time::Duration::from_millis(400));
    seq.ok("uploaded 4.2 MB"); // Automatically starts "Restarting server"
    
    std::thread::sleep(std::time::Duration::from_millis(400));
    seq.ok("online at port 8080"); // Automatically completes the root task
}
```
