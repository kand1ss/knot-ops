# Transport Benchmarks Analysis

## System Configuration
- **OS**: Windows 10 Pro
- **CPU**: Intel Celeron N2840 (Dual-core, ~2.16 GHz)
- **RAM**: 4 GB (DDR3)
- **IPC Mechanism**: Named Pipes (Windows)

## Benchmark Results Summary

### Core Performance
| Benchmark | Variant | Latency (Avg) | Throughput / Speed | Notes |
|-----------|---------|---------------|-------------------|-------|
| Sequential Roundtrip | Binary | ~446 µs | - | Base request-response latency |
| Sequential Roundtrip | JSON | ~399 µs | - | High jitter on this hardware |
| Lifecycle | Connect + Ping + Drop | ~1.29 ms | - | Connection setup overhead |
| Framing Overhead | Messaged | ~459 µs | - | Full `MessageTransport` |
| Framing Overhead | Raw | ~348 µs | - | Pure `send_frame`/`recv_frame` |
| Middleware Cost | 10 layers | ~510 µs | - | ~13 µs overhead per layer |

### Throughput & Scaling
| Benchmark | Payload Size | Latency (Avg) | Throughput | Mode |
|-----------|--------------|---------------|------------|------|
| Payload Scaling | 1KB | ~506 µs | 1.93 MiB/s | Req/Res |
| Payload Scaling | 256KB | ~9.34 ms | 26.8 MiB/s | Req/Res |
| Payload Scaling | 1MB | ~51.0 ms | 19.6 MiB/s | Req/Res |
| Payload Scaling | 8MB | ~437.0 ms | 18.3 MiB/s | Req/Res |
| Unidirectional | 1KB | ~212 µs | 4.60 MiB/s | Fire-and-forget |
| Unidirectional | 256KB | ~4.98 ms | 50.2 MiB/s | Fire-and-forget |B
| Unidirectional | 4MB | ~141.6 ms | 28.3 MiB/s | Fire-and-forget |

### Concurrency (Small Payloads)
| Level | Latency per Req | Total Throughput | Status |
|-------|-----------------|------------------|--------|
| 10 | ~418 µs | 23.9 Kelem/s | Stable |
| 100 | ~441 µs | 226 Kelem/s | Optimal |
| 500 | ~258 µs | 1.93 Melem/s | Peak Performance |
| 1000 | ~145 ms | 6.9 Kelem/s | **Collapsed** (Resource Contention) |

## Observations & Performance Analysis

### 1. Connection & Framing Overhead
- **Setup Cost**: Establishing a new connection (`~1.29 ms`) is roughly equivalent to 3 sequential roundtrips. For CLI tools, a fresh connection per request is perfectly acceptable if the request itself takes >10ms.
- **Framing Cost**: The high-level `MessageTransport` adds about **~110 µs** of overhead compared to raw byte-frame transport. This includes serialization, correlation ID management, and internal synchronization (inbox mutex).

### 2. Throughput Characteristics
- **Unidirectional vs Bidirectional**: Fire-and-forget (`emit`) is **~2x faster** than request-response for small to medium payloads. This confirms that the IPC pipe itself is fast, and the bottleneck in request-response is the wait time for the peer to process and reply.
- **Peak Throughput**: The system saturates around **50 MiB/s** for unidirectional streams. Beyond 1-4MB payloads, throughput drops as memory bandwidth and cache pressure on the Celeron N2840 become significant.

### 3. Middleware Impact
- Chaining 10 layers of "passthrough" middleware adds only **~100-130 µs** total. This means each middleware layer costs approximately **10-15 µs**. For most applications, adding 3-5 layers of logging/metrics will have negligible impact compared to the base IPC latency.

### 4. Concurrency Limits
- The system scales impressively well up to **500 concurrent requests**, reaching nearly **2 million messages per second** for tiny payloads.
- **The Breaking Point**: At **1000 concurrent requests**, performance collapses. This is likely due to Windows Named Pipe limits, context switching overhead, and task scheduling contention on a dual-core CPU.

## Recommendations
1. **Connection Strategy**: CLI tools should use a fresh connection for simple tasks. For complex multi-step interactions, keep the connection open to avoid the `1.3ms` setup cost.
2. **Optimal Concurrency**: Keep concurrent operations between **50 and 200** for best stability. Avoid exceeding 500.
3. **Payload Management**: Use `emit` (unidirectional) for large data transfers where possible. Keep individual frames under **4MB** to stay in the high-performance zone.
4. **Middleware**: Feel free to use Middleware for observability; the overhead is extremely low.
