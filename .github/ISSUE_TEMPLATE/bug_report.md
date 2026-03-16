---
name: Bug report
about: Create a report to help us improve
title: "[BUG]"
labels: 'type: bug'
assignees: kand1ss

---

### 🐛 Bug Description
**A clear and concise description of what the bug is.**
*Example: The daemon crashes with a 'Broken pipe' error when reloading a TOML config with more than 50 services.*

---

### 🛠 Environment Information
**Context helps in debugging system-level issues:**
- **OS:** (e.g., Fedora 40 / Windows 11)
- **Version/Tag:** (e.g., v0.1-alpha or specific commit hash)
- **Rust Version:** `rustc --version`
- **Component:** (e.g., Core, CLI, Transport)

---

### 👣 Steps to Reproduce
1. Go to '...'
2. Run command '...'
3. See error '...'

---

### 📊 Expected vs. Actual Behavior
**Expected:** What should have happened.
**Actual:** What actually happened (include error messages or exit codes).

---

### 📜 Logs & Traces
**Copy-paste any relevant logs or backtraces here:**
```rust
// Paste your cargo run output or panic backtrace here
```

---

### 📍 Roadmap Context
**Which part of the architecture is failing?**
- **Affected Block:** (e.g., [v0.3 Reliability] -> Crash Recovery)
*Check the roadmap image in the Wiki to identify the failing component.*

📎 Possible Fix
**Optional:** If you have an idea of why this is happening (e.g., "missing Mutex guard in Event Bus").
