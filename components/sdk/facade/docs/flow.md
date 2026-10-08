```mermaid
flowchart TD
    Client[KnotClient::connect] --> QArtifacts{Daemon socket and lock files exist?}
    QArtifacts -->|No| Offline[OfflineHandle]
    QArtifacts -->|Yes| QConnection{gRPC socket connection state?}
    QConnection -->|Healthy| Connected[ConnectedHandle]
    QConnection -->|Failed / Refused| QProcess{Process table PID verification?}
    QProcess -->|Alive / Valid knotd| Kill[KillHandle]
    QProcess -->|Dead / Mismatch| Stale[StaleHandle]
    Kill -->|kill process| Stale
    Stale -->|clean volatile files| Offline
    Offline -->|launch daemon| Connected
    Connected --> QHandshake{Daemon handshake state?}
    QHandshake -->|InSync| Control[ControlHandle / Ready]
    QHandshake -->|OutOfSync| Uncommitted[UncommittedHandle]
    Uncommitted -->|commit / discard| Control
    Control -->|recheck| QHandshake
    Control -->|Execute up/down| Task[TaskHandle]
    Task -->|cancel task| Control
```