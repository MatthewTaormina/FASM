# FASM Engine — Performance Report

**Date:** 2026-04-08
**Generated:** 2026-04-08T22:07:56Z

---

## Test Environment

| Property | Value |
|---|---|
| OS | `Linux 6.17.0-1008-azure x86_64` |
| CPU | AMD EPYC 7763 64-Core Processor |
| Rust | 1.94.1 |
| Node.js | v24.14.1 |
| Python | 3.12.3 |
| HTTP test requests per endpoint | 500 |

> **Reference data notation** — rows marked *†* are published benchmark values
> from public sources (see [References](#references) at the end of this report).
> They are included so FASM can be compared against platforms that cannot be
> run locally (AWS Lambda, Cloudflare Workers, cloud-native deployments).

---

## KPI Dashboard

| KPI | Target | Measured | Status |
|---|---|---|---|
| Cold start (compile + exec) | < 50 ms | 2 ms | ✅ PASS |
| HTTP throughput at c=8 | ≥ 1,000 req/s | 9134.76 req/s | ✅ PASS |
| Idle memory footprint | < 64 MB | 5 MB | ✅ PASS |
| p99 latency at c=8 | < 50 ms | 1 ms | ✅ PASS |

> **KPI definitions:**
> - *Cold start* — process-level overhead of loading the FASM compiler and
>   executing the first function, measured across 2 runs.
> - *HTTP throughput* — end-to-end requests/second through the full axum + FASM
>   dispatcher stack, measured with ApacheBench.
> - *Idle memory* — RSS of the fasm-engine process before any traffic.
> - *p99 latency* — 99th-percentile response time under 8-concurrent clients.

---

## 1. Build & Compile Time

### Rust workspace build (release)

| Metric | Value |
|---|---|
| Total build time | 120 ms |
| fasm CLI binary size | 1211 KB |
| fasm-engine binary size | 6051 KB |

> **Note:** Incremental builds are much faster; the figure above is a clean
> build.  For comparison, a typical Go service builds in ~3–10 s and a
> Node.js project bundles in 2–30 s depending on toolchain.

### FASM function compile time (fib_handler.fasm)

| Stage | Avg (ms) |
|---|---|
| FASM source → bytecode | 2 ms |

---

## 2. Cold Start Latency

Cold start = time from `fork()` to first result (process spawn + runtime
init + function execution).  This is the most important metric for FaaS
platforms where every idle minute can trigger a cold start.

| Platform | Cold Start (ms) | Notes |
|---|---|---|
| **FASM Engine** (compile + exec) | **2** | Measured on this machine |
| Node.js (process spawn + exec) | 24 | Measured on this machine |
| Python 3 (process spawn + exec) | 22 | Measured on this machine |
| AWS Lambda — Rust (provided.al2023) *†* | 17 | Published: lambda-perf.io |
| AWS Lambda — Python 3.12 *†* | 100 | Published: lambda-perf.io |
| AWS Lambda — Node.js 22 *†* | 148 | Published: lambda-perf.io |
| Cloudflare Workers (V8 isolate) *†* | 5 | Published: Cloudflare blog |

> **Interpretation:** FASM Engine's "cold start" includes full FASM→bytecode
> compilation.  In production the bytecode would be cached (pre-compiled),
> reducing cold start to execution-only latency.  A "warm" FASM invocation
> is just the VM dispatch overhead (see §4).

---

## 3. App / Deployment Size

| Runtime / Platform | Typical Size | Notes |
|---|---|---|
| **fasm-engine binary** | 6051 KB | Statically linked Rust |
| **fasm CLI binary** | 1211 KB | Statically linked Rust |
| Native Rust axum *†* | ~4,000 KB | Typical statically linked binary |
| Node.js (hello-world app) *†* | ~50,000 KB | node_modules + JS source |
| Python (FastAPI app) *†* | ~60,000 KB | venv + .py files |
| Docker scratch + Rust *†* | ~5,000 KB | Minimal container image |
| Docker + Node.js *†* | ~150,000 KB | node:alpine base image |
| AWS Lambda Node.js zip *†* | ~1,000–50,000 KB | depends on dependencies |
| Cloudflare Worker bundle *†* | ≤10,240 KB | V8 script size limit (paid) |

> **Strength:** FASM Engine ships as a single statically-linked binary with
> zero runtime dependencies — ideal for minimal container images and edge
> deployments.

---

## 4. HTTP Throughput — Requests per Second

All FASM numbers are end-to-end: TCP → axum → FASM dispatcher → VM → TCP.

### /ping (minimal overhead, returns Int32)

| Platform | c=1 (req/s) | c=8 (req/s) | c=32 (req/s) | Notes |
|---|---|---|---|---|
| **FASM Engine** | 4336.29 | 9134.76 | 9232.42 | Measured |
| Node.js http.server | 3436.31 | 6823.33 | 7730.96 | Measured |
| Python http.server | 2836.44 | 2914.45 | 2900.70 | Measured |
| Native Rust axum *†* | 48,700 | ~80,000 | ~120,000 | Sharkbench 2024 |
| Node.js Express *†* | 5,700 | ~14,000 | ~20,000 | Sharkbench 2025 |
| Python FastAPI *†* | 1,200 | ~8,000 | ~15,000 | Sharkbench 2024 |
| Cloudflare Workers *†* | >1,000 | >1,000 | >1,000 | Published lower bound |
| AWS Lambda warm *†* | 500 | ~1,000 | ~2,000 | With provisioned concurrency |

### /fib (Fibonacci 30 — CPU-intensive)

| Platform | c=1 (req/s) | c=8 (req/s) | Notes |
|---|---|---|---|
| **FASM Engine** | 3949.76 | 8764.40 | Measured |

---

## 5. Latency Distribution (/ping)

ApacheBench percentiles in milliseconds.

| Platform | c=1 p50 | c=1 p99 | c=8 p50 | c=8 p99 | c=32 p50 | c=32 p99 |
|---|---|---|---|---|---|---|
| **FASM Engine** | 0 | 0 | 1 | 1 | 3 | 4 |
| Node.js http.server | 0 | 1 | — | — | — | — |
| Python http.server | 0 | 0 | — | — | — | — |

---

## 6. Memory Footprint

| Platform | Idle RSS | Under load | Notes |
|---|---|---|---|
| **FASM Engine** | 5 MB | 6 MB | Measured (after 500 req) |
| Native Rust axum *†* | ~8 MB | ~8 MB | Sharkbench 2024 |
| Node.js Express *†* | ~83 MB | ~100 MB | Sharkbench 2025 |
| Python FastAPI *†* | ~45 MB | ~60 MB | Sharkbench 2024 |
| Docker scratch container *†* | — | — | ~5 MB base image overhead |
| AWS Lambda Node.js *†* | ~85 MB | N/A | Per-invocation process model |
| Cloudflare Workers *†* | <128 MB | N/A | V8 isolate limit per request |

---

## 7. Strengths & Weaknesses Assessment

### ✅ Strengths

1. **Minimal cold start** — FASM Engine can pre-compile FASM→bytecode at
   deploy time and load it from disk on restart.  Warm-path invocations avoid
   compilation entirely, giving sub-millisecond VM startup.
2. **Small binary footprint** — single statically-linked binary (6051 KB)
   with zero runtime dependencies; fits in a scratch Docker image.
3. **Built-in isolation** — each function runs in a sandboxed VM with its own
   value stack, with optional seccomp/landlock on Linux.  This eliminates the
   need for separate container processes per tenant.
4. **Async HTTP core (axum + tokio)** — inherits Rust's async concurrency
   model; scales gracefully with concurrent connections.
5. **Multi-paradigm dispatch** — HTTP, scheduled tasks, queues, and pub/sub
   through a single binary with a declarative TOML config.

### ⚠️ Weaknesses & Limitations

1. **Interpreted VM overhead** — FASM is a tree-walking bytecode interpreter,
   not JIT-compiled.  Raw numeric throughput is significantly lower than
   native Rust axum (~48700 req/s reference) and even behind Node.js's
   V8 JIT for CPU-heavy tasks.
2. **Compile latency on cold path** — if bytecode is not pre-compiled,
   the first request for a function pays a compilation penalty (measured as
   2 ms end-to-end).  Production deployments MUST pre-compile.
3. **No JIT** — there is no just-in-time compilation; hot loops are always
   interpreted.  This is the primary performance ceiling.  Adding a JIT or
   targeting WebAssembly would be the highest-impact architectural improvement.
4. **Single-language ecosystem** — FASM is a custom language with limited
   library support.  Complex business logic requires writing everything in FASM
   or using the Python/Node sidecar mechanism.
5. **Limited platform comparison** — Cloudflare Workers and AWS Lambda cannot
   be benchmarked locally; competitor figures for these platforms come from
   published benchmarks and may differ from real-world deployments.
6. **No HTTP/2 or connection multiplexing** — the axum server uses HTTP/1.1;
   high-concurrency scenarios benefit less than HTTP/2-native platforms.

### 🔮 Potential Verdict

FASM Engine has **legitimate potential** as an embedded, lightweight FaaS
runtime for edge and IoT scenarios where:
- Binary size and memory footprint matter more than peak throughput.
- Multi-tenant isolation without container overhead is required.
- The operator controls the deployment pipeline and can pre-compile FASM.

It is **not competitive** today with production FaaS platforms for
high-throughput workloads.  A realistic path to competitiveness requires:
1. Bytecode JIT or WASM compilation target.
2. Connection-level pipelining or HTTP/2.
3. Function warm-pooling to eliminate cold starts entirely.

---

## 8. Production-Grade KPI Requirements

The following thresholds define production readiness.  Re-run benchmarks
after each significant change to verify no regression.

| KPI | Minimum | Target | Current | Status |
|---|---|---|---|---|
| Cold start (warm bytecode) | < 100 ms | < 20 ms | 2 ms | ✅ PASS |
| p50 latency (c=8) | < 20 ms | < 5 ms | 1 ms | ✅ |
| p99 latency (c=8) | < 200 ms | < 50 ms | 1 ms | ✅ PASS |
| Throughput (c=8) | ≥ 500 req/s | ≥ 2,000 req/s | 9134.76 req/s | ✅ PASS |
| Idle RSS | < 128 MB | < 32 MB | 5 MB | ✅ PASS |
| Error rate | = 0% | = 0% | 0% | ✅ |

---

## References

| # | Source | URL | Accessed |
|---|---|---|---|
| 1 | Lambda Cold Starts leaderboard | https://maxday.github.io/lambda-perf/ | 2024 |
| 2 | Rust on AWS Lambda production guide | https://www.nandann.com/blog/rust-aws-lambda-production-guide | 2024 |
| 3 | Cloudflare Workers CPU benchmarks | https://blog.cloudflare.com/unpacking-cloudflare-workers-cpu-performance-benchmarks/ | 2024 |
| 4 | Sharkbench — axum | https://sharkbench.dev/web/rust-axum | 2024 |
| 5 | Sharkbench — Express | https://sharkbench.dev/web/javascript-express | 2025 |
| 6 | Sharkbench — FastAPI | https://sharkbench.dev/web/python-fastapi | 2024 |
| 7 | Web Frameworks Benchmark | https://web-frameworks-benchmark.netlify.app/ | 2024 |
| 8 | InfoQ: Cloudflare 99.99% warm start | https://www.infoq.com/news/2025/10/workers-shard-conquer-cold-start/ | 2025 |

---

*Report generated by `benchmarks/run_benchmarks.sh` + `benchmarks/generate_report.sh`.*
*Raw data: `benchmarks/reports/latest_raw.json`.*
