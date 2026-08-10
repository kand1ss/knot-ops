# Knot Client Architecture (`knot-client`)

The `knot-client` crate provides a high-level, type-safe Rust client library for interacting with the `knot` background daemon. It encapsulates IPC connection lifecycle management, workspace state evaluation, process orchestration, and streaming gRPC command execution behind a compile-time enforced **Typestate / Handle Pattern**.

---

## 1. Architectural Overview & Design Rationale

### The Handle Pattern (Typestate State Machine)

Communicating with a background daemon over Unix Domain Sockets (UDS) involves complex state spaces (e.g., missing socket, dead process with residual lock files, un-synchronized workspace manifest, hung gRPC channel). Traditional monolithic client structs rely on runtime boolean checks or deferred error returns when methods are called out of order (e.g., invoking `sync()` when the daemon is not running).

`knot-client` eliminates invalid state operations at **compile time** using Rust's ownership and typestate pattern. Each state in the daemon lifecycle is encapsulated by a dedicated **Handle** struct. Methods on handles consume `self` by value, performing valid state transitions and returning the next handle or state enum.

#### Key Architectural Trade-Offs

| Dimension | Typestate / Handle Pattern (Chosen) | Monolithic Client Struct (Rejected) |
| :--- | :--- | :--- |
| **Compile-Time Safety** | **Guaranteed**. Invalid operations (e.g., running `up` on an offline daemon) fail compilation. | Low. Requires runtime state flags and returns `Error::InvalidState` at runtime. |
| **API Usability** | **Explicit & Self-Documenting**. Autocomplete only presents valid methods for the current state. | High initial simplicity, but prone to runtime misuse and surprise panics/errors. |
| **Memory & Overhead** | Zero runtime memory footprint. Handles wrap transport primitives (`Channel`) or paths without indirection. | Requires `RwLock<State>` synchronization overhead inside the client struct. |
| **State Rigidity** | High. State transitions must be explicitly handled by the caller via pattern matching or chaining. | Low. Dynamic state mutations can occur out-of-order internally. |

---

## 2. Daemon Lifecycle & Connection Flow

The bootstrap entry point for `knot-client` is `KnotClient::connect()` (or via `ClientBuilder::default().connect()`). Primary daemon connection is decoupled from workspace initialization; it inspects the user-session runtime directory (`daemon_runtime_dir()`) for active lock and IPC socket files (`knotd.lock`, `knot.sock`), inspects the OS process table, and evaluates socket responsiveness to determine the exact `ConnectState`.

Workspace context (`WorkspaceMetadata` and `WorkspaceManifest`) is passed downstream during the `handshake()` phase once a `ConnectedHandle` is established.

### Lifecycle Decision Tree
Can be found at [flow.md](flow.md).

### State Machine Transition Model
Can be found at [states.md](states.md)

---

## 3. Handle Taxonomy & Operation Reference

`knot-client` exposes seven specialized handle types across distinct lifecycle phases:

### Phase 1: Connection & Discovery Handles

#### 1. `OfflineHandle`
* **Condition**: The IPC socket is absent or the daemon process is not running in the runtime directory.
* **Key Fields**: `dir: PathBuf`, `daemon_launcher: Box<dyn DaemonLauncher + Send + Sync>`, `policy: Arc<PolicyConfig>`
* **Valid Operations**:
  * `async fn launch(self) -> Result<ConnectedHandle, ClientError>`: Spawns the daemon process via `DaemonLauncher`, retries socket connection up to 40 times (50ms interval) until passing health check, and transitions to `ConnectedHandle`.

#### 2. `StaleHandle`
* **Condition**: Incomplete runtime artifacts exist in `daemon_runtime_dir()` (e.g., lock file exists while socket is missing, or vice versa), or both artifacts exist but the daemon process is dead / connections are refused.
* **Key Fields**: `dir: PathBuf`, `daemon_launcher: Box<dyn DaemonLauncher + Send + Sync>`, `policy: Arc<PolicyConfig>`
* **Valid Operations**:
  * `async fn clean(self) -> std::io::Result<OfflineHandle>`: Unlinks stale socket and PID files from disk and returns an `OfflineHandle` ready for spawning.

#### 3. `KillHandle`
* **Condition**: The daemon socket is hung/unresponsive, but the PID lock file is actively locked by an uncooperative or frozen process.
* **Key Fields**: `dir: PathBuf`, `process: Box<dyn ProcessControl>`, `daemon_launcher: Box<dyn DaemonLauncher + Send + Sync>`, `policy: Arc<PolicyConfig>`
* **Valid Operations**:
  * `fn kill(self) -> Result<StaleHandle, ClientError>`: Synchronously sends termination signals (`SIGKILL`/`SIGTERM`) to the process ID and transitions to `StaleHandle` for file cleanup.

#### 4. `ConnectedHandle`
* **Condition**: IPC channel established successfully over gRPC / UDS, but workspace handshake has not yet occurred.
* **Key Fields**: `client: DaemonServiceClient<Channel>`, `policy: Arc<PolicyConfig>`
* **Valid Operations**:
  * `async fn handshake(self, meta: WorkspaceMetadata, manifest: WorkspaceManifest) -> Result<DaemonSession, ClientError>`: Performs gRPC handshake with daemon. Evaluates server response and returns `DaemonSession::Ready(ControlHandle)` or `DaemonSession::Unsynced(UnsyncedHandle)`.

---

### Phase 2: Session & Control Handles

#### 5. `UnsyncedHandle`
* **Condition**: Handshake succeeded, but the daemon reported the workspace configuration state as `OutOfSync`.
* **Key Fields**: `controller: ControlHandle`
* **Valid Operations**:
  * `async fn sync(self, config: WorkspaceManifest) -> Result<(ControlHandle, CommandHandle<SyncResponse>), ClientError>`: Pushes workspace manifest to daemon, returning the upgraded `ControlHandle` alongside the initial `CommandHandle<SyncResponse>`.

#### 6. `ControlHandle`
* **Condition**: Workspace is registered and in-sync with the daemon. Primary operational handle.
* **Key Fields**: `workspace_meta: WorkspaceMetadata`, `client: DaemonServiceClient<Channel>`, `policy: Arc<PolicyConfig>`
* **Valid Operations**:
  * `async fn up(&self, request: UpRequest) -> Result<CommandHandle<UpResponse>, ClientError>`: Initiates service provisioning pipeline.
  * `async fn down(&self, request: DownRequest) -> Result<CommandHandle<DownResponse>, ClientError>`: Initiates service teardown.
  * `async fn status(&self, request: StatusRequest) -> Result<StatusResponse, ClientError>`: Queries real-time service and node statuses.
  * `async fn sync(&self, manifest: WorkspaceManifest) -> Result<CommandHandle<SyncResponse>, ClientError>`: Re-synchronizes manifest.

#### 7. `CommandHandle<E>`
* **Condition**: Active long-running command execution stream on the daemon.
* **Trait Implementations**: `Stream<Item = Result<E, tonic::Status>>`
* **Key Fields**: `command_id: String`, `events: Streaming<E>`, `client: DaemonServiceClient<Channel>`
* **Valid Operations**:
  * `async fn cancel(&mut self, reason: impl Into<String>) -> Result<bool, tonic::Status>`: Sends explicit cancellation signal (`CancelCommandRequest`) with `x-command-id` header to abort daemon execution.

---

## 4. Transport Protocol & IPC Layer

`knot-client` uses **gRPC over Unix Domain Sockets (UDS)** (or Named Pipes on Windows) to communicate with the local `knot` daemon.

* **Transport Abstraction**: Uses `knot_grpc::IpcConnector` to establish a Tokio `UnixStream` underlying a Tonic `Channel` targeting dummy endpoint `http://[::]`.
* **Command Context Propagation**: Every command execution returns an `x-command-id` response header, allowing streaming logs and cancellation requests to be correlated across gRPC calls.
* **Timeout Policies**: Configured via `PolicyConfig` / `TimeoutPolicy` (distinguishes fast RPCs like status/handshake from long-running command streams).

---

## 5. Daemon Discovery & Auto-Launcher Strategy

`ClientBuilder` provides pluggable process spawning through the `DaemonLauncher` trait:

```rust
#[async_trait]
pub trait DaemonLauncher: Send + Sync {
    async fn launch(&self) -> Result<u32, ClientError>;
    fn binary_path(&self) -> &Path;
}
```

### Available Launcher Implementations

1. `DefaultLauncher`: Auto-selects between `StandardDaemonLauncher` (sibling executable in current process dir) and system `PATH`.
2. `StandardDaemonLauncher`: Targets `knotd` binary in relative build/install output directories.
3. `SystemPathLauncher`: Locates `knotd` via OS `PATH` resolution.
4. `ExternalPathLauncher`: Allows explicit path configuration to arbitrary `knotd` binary location.

---

## 6. Error Taxonomy

All errors produced by `knot-client` are mapped into the unified `ClientError` enum:

```rust
pub enum ClientError {
    Workspace(WorkspaceError),
    Daemon(DaemonLifecycleError),
    Protocol(tonic::Status),
    Transport(tonic::transport::Error),
    Io(std::io::Error),
    Contract(String),
}
```

* **`WorkspaceError`**: Workspace resolution/file issues (`NotInitialized`, `BrokenData`).
* **`DaemonLifecycleError`**: High-level lifecycle failures (`NotRunning(PathBuf)`, `LaunchFailed { message, binary_path, error }`).
* **`Protocol`**: Wrapped gRPC `tonic::Status` errors.
* **`Contract`**: Protocol or state assumption violations (e.g., missing `x-command-id` metadata header, unrecognized enum state).

---

## 7. End-to-End Code Example

The following example illustrates connecting to the daemon and navigating state handles to streaming execution events:

```rust
use knot_client::{KnotClient, states::ConnectState, states::DaemonSession};
use knot_proto::commands::v1::UpRequest;
use knot_proto::data::v1::{WorkspaceManifest, WorkspaceMetadata};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Evaluate daemon state & connect
    let connect_state = KnotClient::connect().await?;

    // 2. Resolve offline / stale states to get a ConnectedHandle
    let connected_handle = match connect_state {
        ConnectState::Offline(offline) => {
            println!("Daemon is offline. Launching background process...");
            offline.launch().await?
        }
        ConnectState::Stale(stale) => {
            println!("Stale daemon socket detected. Cleaning up...");
            let offline = stale.clean().await?;
            offline.launch().await?
        }
        ConnectState::Hung(hung) => {
            println!("Daemon process is hung. Terminating...");
            let stale = hung.kill()?;
            let offline = stale.clean().await?;
            offline.launch().await?
        }
        ConnectState::Connected(connected) => connected,
    };

    // 3. Perform gRPC handshake with workspace metadata and manifest
    let metadata = WorkspaceMetadata::default();
    let manifest = WorkspaceManifest::default();
    let session = connected_handle.handshake(metadata, manifest.clone()).await?;

    // 4. Ensure workspace is synced to reach ControlHandle
    let control_handle = match session {
        DaemonSession::Ready(control) => control,
        DaemonSession::Unsynced(unsynced) => {
            println!("Workspace out of sync. Synchronizing...");
            let (control, _sync_stream) = unsynced.sync(manifest).await?;
            control
        }
    };

    // 5. Execute command and stream events
    let mut command_handle = control_handle.up(UpRequest::default()).await?;
    println!("Started 'up' command [ID: {}]", command_handle.command_id);

    while let Some(event) = command_handle.events.next().await {
        match event {
            Ok(evt) => println!("Received event: {:?}", evt),
            Err(err) => {
                eprintln!("Stream error: {}. Cancelling command...", err);
                command_handle.cancel("error during execution stream").await?;
                break;
            }
        }
    }

    Ok(())
}
```
