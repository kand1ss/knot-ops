```mermaid
stateDiagram-v2
    [*] --> Synced

    Synced --> OutOfSync : Config changed
    OutOfSync --> Synced : Apply workspace
```