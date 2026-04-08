# fasm-engine

The FASM FaaS (Function-as-a-Service) engine — a high-performance, async HTTP gateway that dispatches incoming requests to compiled FASM programs running in isolated VM sandboxes.

## Features

- **HTTP routing** — config-driven `(method, path)` → FASM function mapping with `:param` path segments
- **Live deploy API** — upload FASM source or precompiled bytecode into a running engine over HTTP, no restart required
- **Namespaced apps** — dot-notation org namespaces (`com.acme.payments`) containing apps with isolated file stores
- **Back-pressure** — semaphore-bounded concurrency (`max_concurrent`); 503 on overload
- **Message queues** — shared queues with at-least-once delivery, ack/nack, retry cap, and visibility timeout
- **Pub/sub** — fan-out topic registry with per-execution private queues
- **Cron scheduler** — crontab-style job scheduling calling arbitrary FASM functions
- **Prometheus metrics** — `/metrics` (invocations, errors, queue depth, active sandboxes)
- **Plugin autodiscovery** — sidecar plugins auto-loaded from a discovery directory

---

## Quick Start

```toml
# engine.toml
[server]
host = "127.0.0.1"
port = 8080

[engine]
max_concurrent = 64

[storage]
data_dir    = "./data"      # uploaded files + app manifests live here
# admin_token = "secret"   # uncomment to require X-Admin-Token on /api/v1/

# Optional: static routes loaded from config (no upload needed)
[[routes]]
method   = "GET"
path     = "/ping"
function = "Ping"
source   = "ping.fasm"
```

```sh
cargo run -p fasm-engine -- --config engine.toml --dir ./functions
```

---

## Deploying a Cloud Function (Live API)

Functions can be deployed into a running engine without any config change or restart.

### 1. Create a namespace (org)

```sh
curl -X POST http://localhost:8080/api/v1/namespaces \
  -H "Content-Type: application/json" \
  -H "X-Admin-Token: secret" \
  -d '{"name": "com.acme"}'
```

### 2. Create an app

```sh
curl -X POST http://localhost:8080/api/v1/namespaces/com.acme/apps \
  -H "Content-Type: application/json" \
  -d '{"name": "payments"}'
```

### 3. Upload a FASM file

```sh
# Raw source
curl -X PUT http://localhost:8080/api/v1/namespaces/com.acme/apps/payments/files/charge.fasm \
  --data-binary @charge.fasm

# Pre-compiled bytecode
curl -X PUT http://localhost:8080/api/v1/namespaces/com.acme/apps/payments/files/charge.fasmc \
  --data-binary @charge.fasmc

# Gzip-compressed (source or bytecode)
curl -X PUT http://localhost:8080/api/v1/namespaces/com.acme/apps/payments/files/charge.fasm \
  -H "Content-Encoding: gzip" \
  --data-binary @charge.fasm.gz
```

The engine auto-detects `.fasmc` by the `FSMC` magic prefix. Everything else is treated as FASM source.

### 4. Register an entry point (hot-load)

```sh
curl -X POST http://localhost:8080/api/v1/namespaces/com.acme/apps/payments/routes \
  -H "Content-Type: application/json" \
  -d '{
    "method":   "POST",
    "path":     "/pay",
    "function": "Charge",
    "file":     "charge.fasm"
  }'
# → 201 Created  { "id": "<uuid>", "method": "POST", "path": "/pay", ... }
```

Traffic arrives at `POST /pay` immediately. The route is persisted in `data/com.acme/payments/manifest.json` and survives engine restarts.

### 5. Update a function

```sh
# Upload new version
curl -X PUT .../files/charge.fasm --data-binary @charge_v2.fasm

# Remove old route
curl -X DELETE .../routes/<uuid>

# Re-register (in-flight requests on old route finish naturally)
curl -X POST .../routes -d '{"method":"POST","path":"/pay","function":"Charge","file":"charge.fasm"}'
```

### 6. Tear down

```sh
# Remove a single route
curl -X DELETE http://localhost:8080/api/v1/namespaces/com.acme/apps/payments/routes/<uuid>

# Delete whole app (unloads all routes, removes files)
curl -X DELETE http://localhost:8080/api/v1/namespaces/com.acme/apps/payments

# Delete namespace (must be empty)
curl -X DELETE http://localhost:8080/api/v1/namespaces/com.acme
```

---

## Management API Reference

All endpoints live under `/api/v1/`. Auth header `X-Admin-Token` is required on write operations when `admin_token` is configured.

### Namespaces

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/namespaces` | List all namespaces |
| `POST` | `/api/v1/namespaces` | Create namespace `{"name":"com.acme"}` |
| `GET` | `/api/v1/namespaces/:ns` | Get namespace + app list |
| `DELETE` | `/api/v1/namespaces/:ns` | Delete namespace (must be empty) |

### Apps

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/namespaces/:ns/apps` | List apps |
| `POST` | `/api/v1/namespaces/:ns/apps` | Create app `{"name":"checkout"}` |
| `GET` | `/api/v1/namespaces/:ns/apps/:app` | App manifest (files + routes) |
| `DELETE` | `/api/v1/namespaces/:ns/apps/:app` | Delete app + hot-unload all routes |

### Files

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/namespaces/:ns/apps/:app/files` | List uploaded files |
| `PUT` | `/api/v1/namespaces/:ns/apps/:app/files/:filename` | Upload file (replaces if exists) |
| `GET` | `/api/v1/namespaces/:ns/apps/:app/files/:filename` | Download raw file |
| `DELETE` | `/api/v1/namespaces/:ns/apps/:app/files/:filename` | Delete file (rejected if route still references it) |

### Routes (entry points)

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v1/namespaces/:ns/apps/:app/routes` | List active routes |
| `POST` | `/api/v1/namespaces/:ns/apps/:app/routes` | Register route (hot-load, compiles file) |
| `DELETE` | `/api/v1/namespaces/:ns/apps/:app/routes/:id` | Unregister route (hot-unload) |

### Other endpoints

| Endpoint | Description |
|---|---|
| `GET /metrics` | Prometheus-format counters |
| `GET /admin/queues` | Live queue depth as JSON |

---

## Static vs. Managed Routes

| | Static (config) | Managed (API) |
|---|---|---|
| Loaded from | `engine.toml` `[[routes]]` | `POST /api/v1/.../routes` |
| Hot-remove | ✗ | ✓ |
| Persisted | in config file | `data/*/manifest.json` |
| Survives restart | ✓ (via config) | ✓ (via manifest reload) |

Both share the same `RouteTable`. A `409 Conflict` is returned if either type already occupies a `(method, path)`.

---

## Configuration Reference

```toml
[server]
host = "0.0.0.0"   # default
port = 8080         # default

[engine]
max_concurrent = 256   # max parallel FASM executions (default: 256)
clock_hz       = 0     # instruction rate limit, 0 = unlimited

[storage]
data_dir    = "data"   # relative to config file location (default: "data")
admin_token = "..."    # if set, X-Admin-Token required on write endpoints

[plugins]
discovery_dir = "plugins"   # directory scanned for *.plugin.toml sidecars

[[routes]]         # zero or more static routes
method   = "GET"
path     = "/ping"
function = "Ping"
source   = "ping.fasm"

[[schedules]]
name     = "cleanup"
cron     = "0 0 * * *"   # daily at midnight
function = "Cleanup"
source   = "cleanup.fasm"

[[queues]]
name     = "emails"
function = "SendEmail"
source   = "mailer.fasm"
```

---

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

```sh
cargo bench -p fasm-engine
# HTML reports → target/criterion/
```

---

## Testing

```sh
# Unit tests (metrics, queues, pubsub, router + dynamic route tests)
cargo test -p fasm-engine --lib

# Integration tests (HTTP routing, 404, overload, concurrency)
cargo test -p fasm-engine --test engine_integration_test

# Load + memory test (50 callers × 100 req, P99 < 2s, RSS delta < 256MB)
cargo test -p fasm-engine --test load_test -- --nocapture --ignored
```

---

## Architecture

```
Deploy flow (Management API)                 Serving flow
────────────────────────────                 ────────────
PUT  /api/v1/.../files/fn.fasm               HTTP request
  → stored to data/ns/app/files/                 │
                                            axum fallback handler
POST /api/v1/.../routes                         │  pack $args struct
  → compile_source() / decode_program()     RwLock<RouteTable>::match_route()
  → RwLock<RouteTable>::add_route_dyn()         │  Arc<Program> cloned out
  → persist manifest.json                   TaskDispatcher::spawn_async()
                                                │  semaphore guard
                                            Executor::run_named(program, func, $args)
                                                │
                                            Value → JSON response
```

---

## FASM Handler Convention

Any FASM function can serve as an HTTP handler. The engine builds a `$args`
STRUCT from the request and passes it as the sole argument.

### `$args` layout

| Field key | Source | FASM type |
|-----------|--------|-----------|
| 0, 1, … | Path parameters (`:id`, `:slug`, …) | `VEC<UINT8>` — UTF-8 bytes |
| next | Query string values (`?k=v`, one per pair) | `VEC<UINT8>` — percent-decoded |
| last | Request body | See type table below |

### JSON body → FASM type mapping

When `Content-Type: application/json` the body is fully translated into FASM
types before the function is called:

| JSON | FASM `Value` | Notes |
|------|-------------|-------|
| `{"x":1,"y":2}` | `STRUCT{0→INT32(1), 1→INT32(2)}` | String keys dropped; positional |
| `[1,"hi",true]` | `VEC[INT32, VEC<UINT8>, BOOL]` | Recursive |
| `"hello"` | `VEC<UINT8>` | Bare bytes, no quotes |
| `42` | `INT32` (`INT64` if > 2 147 483 647) | |
| `3.14` | `FLOAT64` | |
| `true` / `false` | `BOOL` | |
| `null` | `NULL` | |
| `{"$b64":"aGk="}` | `VEC<UINT8>` | Binary round-trip sentinel |

Any other `Content-Type` passes the raw body bytes as `VEC<UINT8>`.

### FASM return value → JSON response

| FASM `Value` | JSON |
|-------------|------|
| `VEC<UINT8>` valid UTF-8 | `"string"` |
| `VEC<UINT8>` binary | `{"$b64":"<base64-std>"}` |
| `VEC[…]` mixed | `[…]` |
| `STRUCT{0→v, 1→v}` | `{"0":…, "1":…}` |
| `INT32` / `INT64` / `FLOAT64` | number |
| `BOOL` | `true` / `false` |
| `NULL` | `null` |
| `OPTION Some(v)` / `None` | value / `null` |
| `RESULT Ok(v)` | `{"ok": value}` |
| `RESULT Err(code)` | `{"err": code}` |

### Binary data — `$b64` sentinel

When a `VEC<UINT8>` is not valid UTF-8 the engine serialises it as:

```json
{ "$b64": "<standard base64>" }
```

Clients send binary back the same way — the engine recognises the sentinel and
decodes it to `VEC<UINT8>` before calling the function.

### Examples

```fasm
; GET /ping
FUNC Ping
    RET "pong"
ENDF

; GET /echo/:word
FUNC Echo
    LOCAL 0, VEC, word
    GET_FIELD $args, 0, word   ; field 0 = first path param
    RET word
ENDF

; POST /add  body: {"a":3,"b":4}
FUNC Add
    LOCAL 0, INT32, a
    LOCAL 1, INT32, b
    LOCAL 2, INT32, sum
    GET_FIELD $args, 0, a      ; field 0 = first JSON object value
    GET_FIELD $args, 1, b      ; field 1 = second JSON object value
    ADD a, b, sum
    RET sum
ENDF
```
