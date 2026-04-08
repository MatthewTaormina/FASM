# fasm-engine

The FASM FaaS (Function-as-a-Service) engine — a high-performance, async HTTP gateway that dispatches incoming requests to compiled FASM programs running in isolated VM sandboxes.

## Features

- **HTTP routing** — config-driven `(method, path)` → FASM function mapping with `:param` path segments
- **Back-pressure** — semaphore-bounded concurrency (`max_concurrent`); 503 on overload
- **Message queues** — shared queues with at-least-once delivery, ack/nack, retry cap, and visibility timeout
- **Pub/sub** — fan-out topic registry with per-execution private queues
- **Cron scheduler** — crontab-style job scheduling calling arbitrary FASM functions
- **Event bus** — named event channels that fanout to multiple listeners
- **Prometheus metrics** — `/metrics` (invocations, errors, queue depth, active sandboxes)
- **Admin API** — `/admin/queues` returns live queue state as JSON
- **Plugin autodiscovery** — sidecar plugins auto-loaded from a discovery directory

## Quick Start

```toml
# config.toml
[server]
host = "127.0.0.1"
port = 8080

[engine]
max_concurrent = 64

[[routes]]
method   = "GET"
path     = "/ping"
function = "Ping"
source   = "ping.fasm"
```

```sh
cargo run -p fasm-engine -- --config config.toml --dir ./functions
```

## Benchmark Results (debug build, local machine)

| Benchmark | Result |
|---|---|
| `http_ping_roundtrip` | ~102 µs / req |
| Concurrent throughput ×1 | 9.6K req/s |
| Concurrent throughput ×8 | 37.9K req/s |
| Concurrent throughput ×32 | **59.7K req/s** |
| Raw VM — `Ping` (no HTTP) | ~12.5 µs |
| Raw VM — `Fib(30)` | ~34.8 µs |
| HTTP + VM — `Fib(30)` | ~125 µs |

Run benchmarks:

```sh
cargo bench -p fasm-engine
# HTML reports → target/criterion/
```

## Testing

```sh
# Unit tests (metrics, queues, pubsub, router)
cargo test -p fasm-engine --lib

# Integration tests (HTTP routing, 404, overload, concurrency)
cargo test -p fasm-engine --test engine_integration_test

# Load + memory test (50 callers × 100 req, P99 < 2s, RSS delta < 256MB)
cargo test -p fasm-engine --test load_test -- --nocapture --ignored
```

## Architecture

```
HTTP request
    │
    ▼
axum fallback handler (http_handler.rs)
    │  pack path params + body into Value::Struct($args)
    ▼
RouteTable::match_route()   ← compiled program + func name
    │
    ▼
TaskDispatcher::spawn_async()
    │  semaphore guard (back-pressure)
    ▼
tokio::spawn → Executor::run_named(program, func, $args)
    │
    ▼
Value (return) → JSON response
```

## FASM Handler Convention

Any FASM function can be an HTTP handler. Path parameters are injected as `VEC<UINT8>` fields in `$args` (field 0 = first param, etc.):

```fasm
FUNC Echo
    LOCAL 0, VEC, param_bytes
    GET_FIELD $args, 0, param_bytes   // first path param
    RET param_bytes
ENDF
```
