//! Criterion benchmarks for the FASM FaaS engine.
//!
//! Run:
//!   cargo bench -p fasm-engine
//!
//! Produces HTML reports in `target/criterion/`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::runtime::Runtime;

use fasm_compiler::compile_source;
use fasm_engine::{
    config::{EngineConfig, EngineSettings, PluginsConfig, RouteConfig, ServerConfig},
    engine::run_with_listener,
};
use fasm_engine::{
    dispatcher::{ExecRequest, TaskDispatcher},
    metrics::MetricsRegistry,
};
use fasm_jit::FasmJit;
use fasm_sandbox::SandboxConfig;
use fasm_vm::{value::FasmStruct, Value};

// ── HTTP client ─────────────────────────────────────────────────────────────────

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(64)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
}

// ── Engine helpers ────────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {}", name, e))
}

fn make_engine_config(routes: Vec<RouteConfig>) -> EngineConfig {
    EngineConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        engine: EngineSettings {
            max_concurrent: 128,
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

/// Launch the engine in a dedicated background std::thread (own tokio runtime).
/// Returns the `base_url`.  The thread runs for the lifetime of the process.
fn launch_engine_bg(routes: Vec<RouteConfig>) -> String {
    // We need the address before spawning the thread, so bind in the main thread.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().expect("local_addr");

    std::thread::spawn(move || {
        let rt = Runtime::new().unwrap();
        rt.block_on(async move {
            let async_listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let config = make_engine_config(routes);
            let dir = fixtures_dir();
            let _ = run_with_listener(config, dir, async_listener).await;
        });
    });

    // Wait until the engine accepts an HTTP connection
    let url = format!("http://{}", addr);
    for _ in 0..100 {
        if std::net::TcpStream::connect(addr).is_ok() {
            std::thread::sleep(Duration::from_millis(200));
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    url
}

// Shared base URLs (engines live forever in background threads)
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

// ── Benchmark 1: HTTP ping round-trip latency ─────────────────────────────────

fn bench_http_ping_latency(c: &mut Criterion) {
    let base = ping_base();
    let url = format!("{}/ping", base);
    let rt = Runtime::new().unwrap();

    c.bench_function("http_ping_roundtrip", |b| {
        b.to_async(&rt).iter(|| {
            let u = url.clone();
            async move {
                let r = client().get(&u).send().await.unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        });
    });
}

// ── Benchmark 2: Concurrent HTTP throughput ───────────────────────────────────

fn bench_http_concurrent_throughput(c: &mut Criterion) {
    let base = ping_base().to_string();
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("http_concurrent_throughput");

    for concurrency in [1usize, 4, 8, 16, 32] {
        group.throughput(Throughput::Elements(concurrency as u64));
        let b2 = base.clone();
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            &concurrency,
            |b, &n| {
                let b3 = b2.clone();
                b.to_async(&rt).iter(move || {
                    let b4 = b3.clone();
                    async move {
                        let futs: Vec<_> = (0..n)
                            .map(|_| {
                                let u = format!("{}/ping", b4);
                                async move { client().get(&u).send().await.unwrap() }
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

// ── Benchmark 3: Raw VM — ping (no HTTP overhead) ────────────────────────────

fn bench_raw_vm_ping(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let src = read_fixture("ping.fasm");
    let program = Arc::new(compile_source(&src).expect("compile ping.fasm"));
    let metrics = MetricsRegistry::new();
    let sandbox = Arc::new(SandboxConfig::default());
    let disp = TaskDispatcher::new_with_config(128, metrics, sandbox);

    c.bench_function("vm_raw_ping", |b| {
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
                criterion::black_box(d.spawn_async(req).await.unwrap());
            }
        });
    });
}

// ── Benchmark 4: Raw VM — Fibonacci(30) ──────────────────────────────────────

fn bench_raw_vm_fib30(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let src = read_fixture("fib_handler.fasm");
    let program = Arc::new(compile_source(&src).expect("compile fib_handler.fasm"));
    let metrics = MetricsRegistry::new();
    let sandbox = Arc::new(SandboxConfig::default());
    let disp = TaskDispatcher::new_with_config(128, metrics, sandbox);

    c.bench_function("vm_raw_fib30", |b| {
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
                criterion::black_box(d.spawn_async(req).await.unwrap());
            }
        });
    });
}

// ── Benchmark 5: Full HTTP — Fibonacci(30) ───────────────────────────────────

fn bench_http_fib_roundtrip(c: &mut Criterion) {
    let base = fib_base();
    let url = format!("{}/fib", base);
    let rt = Runtime::new().unwrap();

    c.bench_function("http_fib30_roundtrip", |b| {
        b.to_async(&rt).iter(|| {
            let u = url.clone();
            async move {
                let r = client().get(&u).send().await.unwrap();
                assert_eq!(r.status().as_u16(), 200);
            }
        });
    });
}

// ── Benchmark 6: Raw VM — Fibonacci(30) with JIT ─────────────────────────────

fn bench_raw_vm_fib30_jit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let src = read_fixture("fib_handler.fasm");
    let program = Arc::new(compile_source(&src).expect("compile fib_handler.fasm"));
    let jit = FasmJit::compile(&program).map(Arc::new);
    let metrics = MetricsRegistry::new();
    let sandbox = Arc::new(SandboxConfig::default());
    let disp = TaskDispatcher::new_with_config(128, metrics, sandbox);

    c.bench_function("vm_raw_fib30_jit", |b| {
        b.to_async(&rt).iter(|| {
            let d = disp.clone();
            let p = program.clone();
            let j = jit.clone();
            async move {
                let req = ExecRequest {
                    func: "FibHandler".into(),
                    program: p,
                    args: Value::Struct(FasmStruct::default()),
                    trigger: "bench".into(),
                    jit: j,
                };
                criterion::black_box(d.spawn_async(req).await.unwrap());
            }
        });
    });
}

criterion_group!(
    benches,
    bench_http_ping_latency,
    bench_http_concurrent_throughput,
    bench_raw_vm_ping,
    bench_raw_vm_fib30,
    bench_raw_vm_fib30_jit,
    bench_http_fib_roundtrip,
);
criterion_main!(benches);
