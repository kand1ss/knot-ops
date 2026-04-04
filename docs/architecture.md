# Architecture

## Overview
Knot follows a daemon/client architecture where a background
process manages services and CLI communicates with it via IPC.


## Crates
- **knot-core** — domain types, errors, utilities.
- **knot-transport** — IPC protocol, message types, middleware.
- **knot-daemon** — background process. Manages services, checks health.
- **knot-cli** — main client. Speaks with daemon via IPC protocol.

## Transport Layer

### Design Philosophy

The transport layer is built on **three core abstractions**:

1. **[`TransportSpec`](src/transport/src/transport/traits.rs#L28)** — Trait bundle linking Request, Response, and Event types with a compatible Codec. Enables type-safe protocol definition without runtime coupling.

2. **[`RawTransport`](src/transport/src/transport/traits.rs#L57)** — Low-level byte-oriented I/O abstraction. Handles frame integrity via length-prefixed protocol. Decouples protocol from underlying IPC (Unix sockets, TCP, mock).

3. **[`MessageTransport`](src/transport/src/transport.rs#L89)** — High-level typed RPC engine. Manages request-response correlation, multiplexing, and middleware pipeline.

This layering ensures:
- **Type safety**: Request/Response/Event types are compile-time checked.
- **Testability**: Mock transports can be swapped without changing business logic.
- **Extensibility**: New codecs or IPC backends require minimal changes.

### Core Abstractions

#### [`Message<Req, Res, Ev>`](src/transport/src/messages.rs#L38)

Universal envelope for all communication:
```rust
pub struct Message<Req, Res, Ev> {
    pub id: u32,              // Correlation ID for request-response matching
    pub timestamp: u64,       // Unix ms at creation time
    pub kind: MessageKind<Req, Res, Ev>,  // Request | Response | Event
    pub metadata: MetadataMap, // Key-value pairs (Cow<'static, str>)
}
```

**Design note:** Uses `Cow<'static, str>` for zero-copy storage of static metadata keys (e.g., `"trace_id"`) while supporting dynamic strings (`String`) when needed.

#### [`MessageKind<Req, Res, Ev>`](src/transport/src/messages.rs#L63)

Three-variant enum representing message roles:
```rust
pub enum MessageKind<Req, Res, Ev> {
    Request(Req),   // Initiated by client, expects correlated Response
    Response(Res),  // Correlated to Request via id
    Event(Ev),      // One-way notification, id = 0
}
```

#### [`MessageContext<'a, R, S>`](src/transport/src/messages/context.rs#L17)

High-level wrapper around incoming message + transport:
```rust
pub struct MessageContext<'a, R, S> {
    transport: &'a MessageTransport<R, S>,
    message: Message<S::Req, S::Res, S::Ev>,
    replied: bool,  // Prevents accidental duplicate replies
}

impl<'a, R, S> MessageContext<'a, R, S> {
    pub async fn reply(&mut self, msg: S::Res, metadata: Option<MetadataMap>) 
        -> Result<(), TransportError>;
    pub async fn emit(&self, msg: Message<...>) 
        -> Result<(), TransportError>;
    pub fn get_meta(&self, key: &str) -> Option<&str>;
    pub fn kind(&self) -> &MessageKind<S::Req, S::Res, S::Ev>;
}
```

Implements `Deref`/`DerefMut` to transparently expose `Message` fields.

### Middleware Pipeline

The transport supports **Chain of Responsibility** middleware for cross-cutting concerns:
```rust
pub trait Middleware<R: RawTransport, S: TransportSpec>: Send + Sync + 'static {
    async fn on_recv(&self, msg: &Message<...>, next: Inbound<'_, R, S>) 
        -> Result<(), TransportError> {
        next.run(msg).await  // Forward to next middleware or handler
    }
    
    async fn on_send(&self, msg: &mut Message<...>, next: Outbound<'_, R, S>) 
        -> Result<(), TransportError> {
        next.run(msg).await
    }
}
```

#### Built-in Middleware

- **`MetricsMiddleware`** — Records per-message/request/event throughput, latency percentiles (min/max/avg via Welford's algorithm), retry counts. Uses atomic counters + `RwLock` for latency state.

Middleware can:
- **Inspect**: Log messages, collect metrics.
- **Mutate**: Add trace IDs, encrypt payloads, compress.
- **Block**: Return error to halt pipeline (e.g., auth failure, rate limit).

### Serialization (Codecs)
```rust
pub trait MessageCodec: Send + Sync + Debug + 'static {
    type Raw;  // Vec<u8>
    
    fn encode<T: Serialize>(message: &T) -> Result<Self::Raw, TransportError>;
    fn decode<T: DeserializeOwned>(raw: Self::Raw) -> Result<T, TransportError>;
}
```

#### Implementations

1. **`BinaryCodec`** (Bincode v3 with config)
   - Little-endian byte order
   - Fixed-int encoding
   - 10 MB limit enforced
   - **Default for production** (minimal overhead, high throughput)

2. **`JsonCodec`** (serde_json)
   - Human-readable
   - **Useful for debugging**, logging, cross-language compatibility
   - ~2-3x larger wire size vs binary

### Protocol Details

#### Frame Format (IPC Transport)
   - **Length prefix**: Ensures frame boundaries without state tracking.
   - **Atomic reads**: Each `recv_frame()` returns exactly one complete message.
   - **Size validation**: Frames exceeding `MAX_MESSAGE_SIZE` (10 MB) are rejected.

#### Request-Response Correlation

1. **Client** sends `Message::request(id, payload)` with unique `id` (from `AtomicU32` counter).
2. **Server** processes and sends `Message::response(id, payload)` with **same `id`**.
3. **Background read loop** matches response `id` against pending map:
   - If match found: routes to waiting `oneshot` channel.
   - If not found: forwards to general inbox (unhandled response or spurious message).
```rust
// Internal state
type PendingMap<S> = HashMap<u32, oneshot::Sender<Message<...>>>;
```

#### Event Broadcasting

1. Events use `id = 0` (not correlated).
2. Sent via `transport.send(Message::event(payload))`.
3. Routed to general inbox for handler consumption.
4. Multiple handlers can subscribe via `transport.recv()` / `transport.serve_with()`.

### API Levels

#### Low-Level: `RawTransport`
```rust
pub trait RawTransport {
    async fn send_frame(&self, frame: &[u8]) -> Result<(), TransportError>;
    async fn recv_frame(&self) -> Result<Vec<u8>, TransportError>;
}
```

Used by transport internals. Rarely called directly by user code.

#### Mid-Level: `MessageTransport`
```rust
pub struct MessageTransport<R, S> { ... }

impl<R, S> MessageTransport<R, S> {
    // Fire-and-forget send
    pub async fn send(&self, msg: Message<...>) -> Result<(), TransportError>;
    
    // Synchronized RPC with full lifecycle
    pub async fn request_full(&self, req: S::Req, timeout: u64, meta: Option<MetadataMap>)
        -> Result<MessageContext<'_, R, S>, TransportError>;
    
    // Simplified: returns only response payload
    pub async fn request(&self, req: S::Req, timeout: u64, meta: Option<MetadataMap>)
        -> Result<S::Res, TransportError>;
    
    // Receive with middleware
    pub async fn recv(&self) -> Result<MessageContext<'_, R, S>, TransportError>;
    
    // Event loop for servers
    pub async fn serve_with<F>(&self, handler: F) -> Result<(), TransportError>
    where F: for<'a> AsyncHandler<'a, R, S>;
    
    // Middleware registration
    pub async fn add_middleware<M: Middleware<R, S>>(&mut self, mw: M);
}
```

#### High-Level: Daemon Protocol Types
```rust
pub struct DaemonTransportSpec;
impl TransportSpec for DaemonTransportSpec {
    type Req = DaemonRequest;
    type Res = DaemonResponse;
    type Ev = DaemonEvent;
    type C = BinaryCodec;
}

pub type DaemonTransport = MessageTransport<IpcTransport, DaemonTransportSpec>;
```

Concrete type alias pre-configured for Knot's daemon protocol. CLI and Daemon use this.

## Usage Example

### CLI (Client)
```rust
use knot_transport::types::DaemonTransport;
use knot_transport::transport::RawTransport;
use knot_transport::transport::ipc::IpcTransport;

let raw = IpcTransport::connect(PathBuf::from("/var/run/knot.sock")).await?;
let client: DaemonTransport = raw.to_messaged();

// Simple request
let response = client.request(
    DaemonRequest::Status,
    30,  // timeout
    None
).await?;

// Full context (with metadata inspection)
let ctx = client.request_full(
    DaemonRequest::Down,
    10,
    None
).await?;

println!("Server response: {:?}", ctx.kind());
```

### Daemon (Server)
```rust
use knot_transport::types::DaemonTransport;
use knot_transport::transport::Server;
use knot_transport::transport::ipc::IpcServer;

let server = IpcServer::bind(PathBuf::from("/var/run/knot.sock")).await?;

server.accept_with(async |transport: DaemonTransport| {
    transport.serve_with(async |mut ctx| {
        match ctx.kind() {
            MessageKind::Request(DaemonRequest::Status) => {
                let services = vec![ /* ... */ ];
                ctx.reply(DaemonResponse::Status { services }, None).await
            }
            MessageKind::Request(DaemonRequest::Down) => {
                ctx.reply(DaemonResponse::Ok, None).await
            }
            _ => Ok(()),
        }
    }).await
}).await?;
```

---

## See Also

- [`src/transport/src/`](src/transport/src/) — Full implementation.
- [`src/transport/tests/integration/`](src/transport/tests/integration/) — Integration test suite.
- [`src/transport/Cargo.toml`](src/transport/Cargo.toml) — Dependencies (`tokio`, `interprocess`, `serde`, `tracing`).