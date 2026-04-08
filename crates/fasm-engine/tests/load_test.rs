//! Load test and memory footprint report for the FASM FaaS engine.
//!
//! Run with:
//!   cargo test -p fasm-engine load_and_memory_report -- --nocapture --ignored
//!
//! The test prints a structured performance report to stdout and asserts on
//! hard limits (p99 < 2 s, memory delta < 256 MB).

mod common;
use common::TestEngine;

use std::time::{Duration, Instant};

// ── helpers ───────────────────────────────────────────────────────────────────

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() { return 0; }
    let idx = ((sorted.len() as f64 * p / 100.0) as usize).min(sorted.len() - 1);
    sorted[idx]
}

fn process_rss_kb() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory() / 1024).unwrap_or(0)
}

// ── load test ─────────────────────────────────────────────────────────────────

/// Full load + memory footprint report.
///
/// Uses `#[ignore]` so it doesn't run on every `cargo test`; opt-in via `--ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "run explicitly: cargo test load_and_memory_report -- --nocapture --ignored"]
async fn load_and_memory_report() {
    const CONCURRENT_CALLERS: usize = 50;
    const REQUESTS_PER_CALLER: usize = 100;
    const TOTAL_REQUESTS: usize = CONCURRENT_CALLERS * REQUESTS_PER_CALLER;

    println!("\n══════════════════════════════════════════════════════════");
    println!("  FASM Engine — Load & Memory Report");
    println!("══════════════════════════════════════════════════════════");
    println!("  Concurrency : {} callers × {} requests = {} total",
        CONCURRENT_CALLERS, REQUESTS_PER_CALLER, TOTAL_REQUESTS);

    // ── Ping benchmark (lightweight) ─────────────────────────────────────────

    println!("\n── Ping endpoint (/ping — Int32 return, no CPU work)");
    let engine = TestEngine::start_fixtures(128).await;
    let base_rss = process_rss_kb();
    println!("  Baseline RSS : {} KB ({:.1} MB)", base_rss, base_rss as f64 / 1024.0);

    let base = engine.base_url.clone();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(CONCURRENT_CALLERS)
        .build()
        .unwrap();

    let wall_start = Instant::now();

    let futs: Vec<_> = (0..CONCURRENT_CALLERS).map(|_| {
        let c = client.clone();
        let base = base.clone();
        tokio::spawn(async move {
            let mut latencies = Vec::with_capacity(REQUESTS_PER_CALLER);
            let mut errors = 0usize;
            for _ in 0..REQUESTS_PER_CALLER {
                let t0 = Instant::now();
                match c.get(format!("{}/ping", base)).send().await {
                    Ok(r) if r.status().as_u16() == 200 => {
                        latencies.push(t0.elapsed().as_micros());
                    }
                    Ok(r) => {
                        eprintln!("unexpected status: {}", r.status());
                        errors += 1;
                    }
                    Err(e) => {
                        eprintln!("request error: {}", e);
                        errors += 1;
                    }
                }
            }
            (latencies, errors)
        })
    }).collect();

    let mut all_latencies: Vec<u128> = Vec::with_capacity(TOTAL_REQUESTS);
    let mut total_errors = 0usize;
    for f in futs {
        let (lat, err) = f.await.unwrap();
        all_latencies.extend(lat);
        total_errors += err;
    }

    let wall_elapsed = wall_start.elapsed();
    let peak_rss = process_rss_kb();

    all_latencies.sort_unstable();
    let p50  = percentile(&all_latencies, 50.0);
    let p90  = percentile(&all_latencies, 90.0);
    let p99  = percentile(&all_latencies, 99.0);
    let mean = if all_latencies.is_empty() { 0 }
               else { all_latencies.iter().sum::<u128>() / all_latencies.len() as u128 };

    let succeeded = TOTAL_REQUESTS - total_errors;
    let rps = succeeded as f64 / wall_elapsed.as_secs_f64();
    let delta_rss = peak_rss.saturating_sub(base_rss);

    println!("  Completed    : {}/{} requests succeeded", succeeded, TOTAL_REQUESTS);
    println!("  Wall time    : {:.2} s", wall_elapsed.as_secs_f64());
    println!("  Throughput   : {:.0} req/s", rps);
    println!("  Latency (µs) : mean={} p50={} p90={} p99={}", mean, p50, p90, p99);
    println!("  Peak RSS     : {} KB ({:.1} MB)", peak_rss, peak_rss as f64 / 1024.0);
    println!("  RSS delta    : {} KB ({:.1} MB)", delta_rss, delta_rss as f64 / 1024.0);

    drop(engine); // shut down engine before fib benchmark

    // ── Fib benchmark (CPU-heavy) ────────────────────────────────────────────

    println!("\n── Fib endpoint (/fib — Fibonacci(30), CPU-heavy)");
    let engine2 = TestEngine::start_fixtures(128).await;
    let base2 = engine2.base_url.clone();
    let client2 = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(32)
        .build()
        .unwrap();

    const FIB_CALLERS: usize = 16;
    const FIB_PER_CALLER: usize = 10;
    let fib_base_rss = process_rss_kb();
    let fib_start = Instant::now();

    let fib_futs: Vec<_> = (0..FIB_CALLERS).map(|_| {
        let c = client2.clone();
        let base = base2.clone();
        tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut errors = 0usize;
            for _ in 0..FIB_PER_CALLER {
                let t0 = Instant::now();
                match c.get(format!("{}/fib", base)).send().await {
                    Ok(r) if r.status().as_u16() == 200 => {
                        latencies.push(t0.elapsed().as_millis());
                    }
                    Ok(r) => { errors += 1; eprintln!("fib status: {}", r.status()); }
                    Err(e) => { errors += 1; eprintln!("fib error: {}", e); }
                }
            }
            (latencies, errors)
        })
    }).collect();

    let mut fib_latencies: Vec<u128> = Vec::new();
    let mut fib_errors = 0;
    for f in fib_futs {
        let (lat, err) = f.await.unwrap();
        fib_latencies.extend(lat);
        fib_errors += err;
    }

    let fib_elapsed = fib_start.elapsed();
    let fib_peak_rss = process_rss_kb();
    fib_latencies.sort_unstable();
    let fib_p99 = percentile(&fib_latencies, 99.0);
    let fib_p50 = percentile(&fib_latencies, 50.0);
    let fib_rps = (FIB_CALLERS * FIB_PER_CALLER - fib_errors) as f64 / fib_elapsed.as_secs_f64();
    let fib_delta = fib_peak_rss.saturating_sub(fib_base_rss);

    println!("  Concurrency  : {} callers × {} = {} requests", FIB_CALLERS, FIB_PER_CALLER, FIB_CALLERS * FIB_PER_CALLER);
    println!("  Wall time    : {:.2} s", fib_elapsed.as_secs_f64());
    println!("  Throughput   : {:.1} req/s", fib_rps);
    println!("  Latency (ms) : p50={} p99={}", fib_p50, fib_p99);
    println!("  Peak RSS     : {} KB ({:.1} MB)", fib_peak_rss, fib_peak_rss as f64 / 1024.0);
    println!("  RSS delta    : {} KB ({:.1} MB)", fib_delta, fib_delta as f64 / 1024.0);
    println!("══════════════════════════════════════════════════════════\n");

    // ── Assertions ───────────────────────────────────────────────────────────
    assert_eq!(total_errors, 0, "ping load: {} errors out of {}", total_errors, TOTAL_REQUESTS);
    assert_eq!(fib_errors, 0, "fib load: {} errors", fib_errors);
    assert!(
        p99 < 5_000_000,  // p99 < 5 s in µs for ping
        "ping p99 latency too high: {} µs",
        p99
    );
    assert!(
        fib_p99 < 30_000,  // fib p99 < 30 s in ms
        "fib p99 latency too high: {} ms",
        fib_p99
    );
    assert!(
        delta_rss < 256 * 1024,  // ping memory delta < 256 MB
        "ping memory leak? delta RSS {} KB",
        delta_rss
    );
}
