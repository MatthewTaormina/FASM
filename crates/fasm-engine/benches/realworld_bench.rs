//! Real-world benchmarks for the FASM FaaS engine.
//!
//! Measures the metrics that matter for production FaaS deployments and
//! compares them against published reference data for Node.js (Express),
//! Python (FastAPI/http.server), native Rust (axum), AWS Lambda, and
//! Cloudflare Workers.
//!
//! # Run
//! ```
//! cargo bench -p fasm-engine --bench realworld_bench
//! ```
//!
//! Results land in `target/criterion/realworld_*/`.

use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

use fasm_compiler::compile_source;
use fasm_engine::{
    config::{EngineConfig, EngineSettings, PluginsConfig, RouteConfig, ServerConfig},
    dispatcher::{ExecRequest, TaskDispatcher},
    engine::run_with_listener,
    metrics::MetricsRegistry,
};
use fasm_sandbox::SandboxConfig;
use fasm_vm::{value::FasmStruct, Value};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(128)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
}

fn make_engine_config(routes: Vec<RouteConfig>) -> EngineConfig {
    EngineConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        engine: EngineSettings {
            max_concurrent: 256,
            hot_reload: false,
            clock_hz: 0,
            enable_seccomp: false,
            enable_landlock: false,
            landlock_allowed_read_paths: vec![],
        },
        plugins: PluginsConfig {
            discovery_dir: None,
        },
        storage: Default::default(),
        routes,
        schedules: vec![],
        queues: vec![],
        events: vec![],
    }
}

/// Spawn the engine in a background thread; return the base URL.
fn launch_engine_bg(routes: Vec<RouteConfig>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();

    std::thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(async move {
            let async_listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let config = make_engine_config(routes);
            let _ = run_with_listener(config, fixtures_dir(), async_listener).await;
        });
    });

    let url = format!("http://{addr}");
    for _ in 0..200 {
        if std::net::TcpStream::connect(addr).is_ok() {
            std::thread::sleep(Duration::from_millis(100));
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    url
}

/// Build a TaskDispatcher pre-loaded with a compiled program.
fn make_dispatcher(source: &str) -> (Arc<fasm_bytecode::Program>, TaskDispatcher) {
    let program = Arc::new(compile_source(source).expect("compile"));
    let metrics = MetricsRegistry::new();
    let sandbox = Arc::new(SandboxConfig::default());
    let disp = TaskDispatcher::new_with_config(256, metrics, sandbox);
    (program, disp)
}

// ── Persistent engine URLs (lazily initialized, live for the process) ─────────

static PING_URL: OnceLock<String> = OnceLock::new();
static FIB_URL: OnceLock<String> = OnceLock::new();

fn ping_base() -> &'static str {
    PING_URL.get_or_init(|| {
        launch_engine_bg(vec![RouteConfig {
            method: "GET".into(),
            path: "/ping".into(),
            function: "Ping".into(),
            source: "ping.fasm".into(),
        }])
    })
}

fn fib_base() -> &'static str {
    FIB_URL.get_or_init(|| {
        launch_engine_bg(vec![RouteConfig {
            method: "GET".into(),
            path: "/fib".into(),
            function: "FibHandler".into(),
            source: "fib_handler.fasm".into(),
        }])
    })
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 1 — Cold Start
//   Measures the time from "raw FASM source string" to "first result produced".
//   This is the closest analogy to a FaaS cold start: no compiled cache,
//   compile then execute.
// ═════════════════════════════════════════════════════════════════════════════

/// Cold start: compile a FASM source then execute the first invocation.
///
/// Competitor reference (approximate, 2024 public benchmarks):
///   - AWS Lambda Rust provided.al2023 : ~17 ms
///   - AWS Lambda Python 3.12          : ~100 ms
///   - AWS Lambda Node.js 22           : ~148 ms
///   - Cloudflare Workers (V8 isolate) : ~5 ms (warm) / up to 100 ms (cold)
///   - Docker container (warm)         : <1 ms (no cold start overhead)
fn bench_cold_start_ping(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let src = read_fixture("ping.fasm");
    let mut group = c.benchmark_group("cold_start");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(50);

    group.bench_function("fasm_compile_and_first_exec_ping", |b| {
        b.to_async(&rt).iter(|| {
            let s = src.clone();
            async move {
                // Compile (simulates JIT / module load)
                let program = Arc::new(compile_source(&s).expect("compile"));
                let metrics = MetricsRegistry::new();
                let sandbox = Arc::new(SandboxConfig::default());
                let disp = TaskDispatcher::new_with_config(4, metrics, sandbox);
                let req = ExecRequest {
                    func: "Ping".into(),
                    program,
                    args: Value::Struct(FasmStruct::default()),
                    trigger: "cold_start".into(),
                    jit: None,
                };
                criterion::black_box(disp.spawn_async(req).await.unwrap())
            }
        });
    });

    group.bench_function("fasm_compile_and_first_exec_fib30", |b| {
        let src_fib = read_fixture("fib_handler.fasm");
        b.to_async(&rt).iter(|| {
            let s = src_fib.clone();
            async move {
                let program = Arc::new(compile_source(&s).expect("compile fib"));
                let metrics = MetricsRegistry::new();
                let sandbox = Arc::new(SandboxConfig::default());
                let disp = TaskDispatcher::new_with_config(4, metrics, sandbox);
                let req = ExecRequest {
                    func: "FibHandler".into(),
                    program,
                    args: Value::Struct(FasmStruct::default()),
                    trigger: "cold_start".into(),
                    jit: None,
                };
                criterion::black_box(disp.spawn_async(req).await.unwrap())
            }
        });
    });

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 2 — Compile / Build Time
//   Time to parse + validate + emit bytecode from source.
//   Lower is better for deployment speed.
// ═════════════════════════════════════════════════════════════════════════════

fn bench_compile_time(c: &mut Criterion) {
    let src_ping = read_fixture("ping.fasm");
    let src_fib = read_fixture("fib_handler.fasm");
    let src_arith = read_fixture("arithmetic.fasm");
    let src_struct = read_fixture("struct_ops.fasm");
    let src_string = read_fixture("string_ops.fasm");

    let mut group = c.benchmark_group("compile_time");
    group.sample_size(200);

    for (name, src) in [
        ("ping", src_ping.as_str()),
        ("fib_handler", src_fib.as_str()),
        ("arithmetic", src_arith.as_str()),
        ("struct_ops", src_struct.as_str()),
        ("string_ops", src_string.as_str()),
    ] {
        group.bench_with_input(BenchmarkId::new("fasm_compile", name), src, |b, s| {
            b.iter(|| criterion::black_box(compile_source(s).expect("compile")));
        });
    }

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 3 — VM Operation Throughput (warm, pre-compiled)
//   Raw operations/second for different workload profiles.
//   This is the apples-to-apples execution throughput without network overhead.
// ═════════════════════════════════════════════════════════════════════════════

fn bench_vm_ops_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("vm_ops_throughput");
    group.throughput(Throughput::Elements(1));
    group.sample_size(500);

    // Ping — minimal overhead, tests dispatch + call + ret path
    {
        let (program, disp) = make_dispatcher(&read_fixture("ping.fasm"));
        group.bench_function("ping_minimal_overhead", |b| {
            b.to_async(&rt).iter(|| {
                let d = disp.clone();
                let p = program.clone();
                async move {
                    let req = ExecRequest {
                        func: "Ping".into(),
                        program: p,
                        args: Value::Struct(FasmStruct::default()),
                        trigger: "bench".into(),
                        jit: None,
                    };
                    criterion::black_box(d.spawn_async(req).await.unwrap())
                }
            });
        });
    }

    // Fibonacci(30) — CPU-intensive iterative loop
    {
        let (program, disp) = make_dispatcher(&read_fixture("fib_handler.fasm"));
        group.bench_function("fibonacci30_cpu_bound", |b| {
            b.to_async(&rt).iter(|| {
                let d = disp.clone();
                let p = program.clone();
                async move {
                    let req = ExecRequest {
                        func: "FibHandler".into(),
                        program: p,
                        args: Value::Struct(FasmStruct::default()),
                        trigger: "bench".into(),
                        jit: None,
                    };
                    criterion::black_box(d.spawn_async(req).await.unwrap())
                }
            });
        });
    }

    // Arithmetic loop (100 iterations of ADD/MUL/DIV)
    {
        let (program, disp) = make_dispatcher(&read_fixture("arithmetic.fasm"));
        group.bench_function("arithmetic_loop_100iters", |b| {
            b.to_async(&rt).iter(|| {
                let d = disp.clone();
                let p = program.clone();
                async move {
                    let req = ExecRequest {
                        func: "ArithLoop".into(),
                        program: p,
                        args: Value::Struct(FasmStruct::default()),
                        trigger: "bench".into(),
                        jit: None,
                    };
                    criterion::black_box(d.spawn_async(req).await.unwrap())
                }
            });
        });
    }

    // Struct set/get operations
    {
        let (program, disp) = make_dispatcher(&read_fixture("struct_ops.fasm"));
        group.bench_function("struct_set_get_ops", |b| {
            b.to_async(&rt).iter(|| {
                let d = disp.clone();
                let p = program.clone();
                async move {
                    let req = ExecRequest {
                        func: "StructOps".into(),
                        program: p,
                        args: Value::Struct(FasmStruct::default()),
                        trigger: "bench".into(),
                        jit: None,
                    };
                    criterion::black_box(d.spawn_async(req).await.unwrap())
                }
            });
        });
    }

    // String operations (alloc, store, eq)
    {
        let (program, disp) = make_dispatcher(&read_fixture("string_ops.fasm"));
        group.bench_function("string_alloc_compare", |b| {
            b.to_async(&rt).iter(|| {
                let d = disp.clone();
                let p = program.clone();
                async move {
                    let req = ExecRequest {
                        func: "StringOps".into(),
                        program: p,
                        args: Value::Struct(FasmStruct::default()),
                        trigger: "bench".into(),
                        jit: None,
                    };
                    criterion::black_box(d.spawn_async(req).await.unwrap())
                }
            });
        });
    }

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 4 — HTTP Request Throughput (end-to-end, sequential)
//   Single-client, sequential HTTP round-trips. Measures the full stack:
//   TCP → axum → FASM dispatcher → VM → response serialization → TCP.
//
//   Competitor reference (single-core, warm):
//     - Native Rust axum "hello world" : ~21,000–48,700 req/s
//     - Node.js Express                : ~5,500–5,800 req/s
//     - Python FastAPI (async)         : ~1,200 req/s (basic JSON endpoint)
//     - Python http.server             : ~800–1,200 req/s
//     (FASM adds VM interpretation overhead on top of axum's raw numbers)
// ═════════════════════════════════════════════════════════════════════════════

fn bench_http_latency_sequential(c: &mut Criterion) {
    let ping_url = format!("{}/ping", ping_base());
    let fib_url = format!("{}/fib", fib_base());
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("http_latency_sequential");
    group.sample_size(500);

    group.bench_function("ping_roundtrip", |b| {
        b.to_async(&rt).iter(|| {
            let u = ping_url.clone();
            async move {
                let r = client().get(&u).send().await.unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        });
    });

    group.bench_function("fib30_roundtrip", |b| {
        b.to_async(&rt).iter(|| {
            let u = fib_url.clone();
            async move {
                let r = client().get(&u).send().await.unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        });
    });

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 5 — Concurrent HTTP Throughput
//   Simulates real FaaS load: N simultaneous callers, each issuing one request
//   in a join_all fan-out.  Throughput = N / wall_time.
//
//   This is the key metric for cloud service comparison:
//     - Cloudflare Workers: >1,000 req/s (light JS logic, V8 isolates)
//     - AWS Lambda (Node 22): ~500–2,000 req/s (with provisioned concurrency)
//     - Docker + Express:  ~5,000–8,000 req/s (warm container, loopback)
//     - Docker + Axum (native Rust): ~20,000–50,000 req/s
// ═════════════════════════════════════════════════════════════════════════════

fn bench_http_concurrent_throughput(c: &mut Criterion) {
    let base = ping_base().to_string();
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("http_concurrent_throughput");

    for &concurrency in &[1usize, 4, 8, 16, 32, 64] {
        group.throughput(Throughput::Elements(concurrency as u64));
        let b_url = base.clone();
        group.bench_with_input(
            BenchmarkId::new("fasm_ping", concurrency),
            &concurrency,
            |b, &n| {
                let u = b_url.clone();
                b.to_async(&rt).iter(move || {
                    let url = u.clone();
                    async move {
                        let futs: Vec<_> = (0..n)
                            .map(|_| {
                                let ping = format!("{url}/ping");
                                async move { client().get(&ping).send().await.unwrap() }
                            })
                            .collect();
                        futures::future::join_all(futs).await;
                    }
                });
            },
        );
    }

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 6 — Concurrent CPU-bound Throughput (Fibonacci)
//   Shows how well the dispatcher parallelises CPU-heavy work using
//   spawn_blocking under the hood.
// ═════════════════════════════════════════════════════════════════════════════

fn bench_http_concurrent_fib(c: &mut Criterion) {
    let base = fib_base().to_string();
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("http_concurrent_fib30");
    group.measurement_time(Duration::from_secs(15));

    for &concurrency in &[1usize, 4, 8, 16] {
        group.throughput(Throughput::Elements(concurrency as u64));
        let b_url = base.clone();
        group.bench_with_input(
            BenchmarkId::new("fasm_fib30", concurrency),
            &concurrency,
            |b, &n| {
                let u = b_url.clone();
                b.to_async(&rt).iter(move || {
                    let url = u.clone();
                    async move {
                        let futs: Vec<_> = (0..n)
                            .map(|_| {
                                let fib = format!("{url}/fib");
                                async move { client().get(&fib).send().await.unwrap() }
                            })
                            .collect();
                        futures::future::join_all(futs).await;
                    }
                });
            },
        );
    }

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 7 — Memory Footprint
//   Records RSS before and after engine startup and after 1,000 ping requests.
//   Reported as a custom benchmark so it appears in Criterion output.
//   (Criterion doesn't track RSS natively; we use a single iteration to
//    capture the snapshot and emit it via a dummy timing measurement.)
// ═════════════════════════════════════════════════════════════════════════════

fn process_rss_kb() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory() / 1024).unwrap_or(0)
}

fn bench_memory_footprint(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let ping_url = format!("{}/ping", ping_base());

    let mut group = c.benchmark_group("memory_footprint");
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);

    // Warm up 100 requests then measure RSS
    group.bench_function("rss_after_100_pings", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let t0 = Instant::now();
                rt.block_on(async {
                    for _ in 0..100 {
                        let _ = client().get(&ping_url).send().await;
                    }
                });
                let rss = process_rss_kb();
                // Encode RSS as microseconds so it shows up in Criterion timing output.
                // The label in the report makes the unit clear.
                total += Duration::from_micros(rss);
                total += t0.elapsed().saturating_sub(t0.elapsed()); // keep t0 used
            }
            total
        });
    });

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Section 8 — Dispatcher Concurrency Scaling
//   How does raw VM throughput scale as we increase the number of concurrent
//   tokio tasks all calling spawn_async at the same time?
// ═════════════════════════════════════════════════════════════════════════════

fn bench_dispatcher_concurrency_scaling(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("dispatcher_concurrency_scaling");
    group.measurement_time(Duration::from_secs(10));

    let src = read_fixture("ping.fasm");
    let (program, disp) = make_dispatcher(&src);

    for &n in &[1usize, 2, 4, 8, 16, 32] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(
            BenchmarkId::new("concurrent_ping_spawns", n),
            &n,
            |b, &count| {
                let d = disp.clone();
                let p = program.clone();
                b.to_async(&rt).iter(move || {
                    let dd = d.clone();
                    let pp = p.clone();
                    async move {
                        let futs: Vec<_> = (0..count)
                            .map(|_| {
                                let ddd = dd.clone();
                                let ppp = pp.clone();
                                async move {
                                    let req = ExecRequest {
                                        func: "Ping".into(),
                                        program: ppp,
                                        args: Value::Struct(FasmStruct::default()),
                                        trigger: "bench".into(),
                                        jit: None,
                                    };
                                    ddd.spawn_async(req).await.unwrap()
                                }
                            })
                            .collect();
                        criterion::black_box(futures::future::join_all(futs).await)
                    }
                });
            },
        );
    }

    group.finish();
}

// ═════════════════════════════════════════════════════════════════════════════
// Registration
// ═════════════════════════════════════════════════════════════════════════════

criterion_group!(
    benches,
    bench_cold_start_ping,
    bench_compile_time,
    bench_vm_ops_throughput,
    bench_http_latency_sequential,
    bench_http_concurrent_throughput,
    bench_http_concurrent_fib,
    bench_memory_footprint,
    bench_dispatcher_concurrency_scaling,
);
criterion_main!(benches);
