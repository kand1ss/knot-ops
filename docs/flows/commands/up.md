```mermaid
flowchart TD
    A[knot up] --> B[Handshake]
    B --> C{Workspace synced?}
    C -->|Yes| E[Execute command]
    C -->|No| D[Sync command]
    D --> E
```