```mermaid
flowchart TD
    A[knot up] --> B[Handshake]
    B --> C{Workspace synced?}
    C -->|Yes| E[Exit command]
    C -->|No| D[Sync command]
```