/// fibbench_native - Native Rust Benchmark
///
/// Fair comparison against `fasm bench examples/fibonacci.fasmc 50000`:
///   - Same algorithm: iterative tail-call-equivalent accumulator
///   - Same N: 30
///   - Same iterations: 50,000
///   - Expected result per iter: 832040 (fib(30))
///
/// Build & run:
///   cargo build --release
///   .\target\release\fibbench_native.exe

use std::time::Instant;

/// Mirrors FASM fibonacci.fasm TAIL_CALL accumulator:
///   Fibonacci(n=30, a=0, b=1) -> iterate down to base case
#[inline(never)]
fn fib_tco(mut n: i32) -> i64 {
    let mut a: i64 = 0;
    let mut b: i64 = 1;
    while n > 1 {
        let next_b = a + b;
        a = b;
        b = next_b;
        n -= 1;
    }
    if n == 0 { a } else { b }
}

fn main() {
    const TARGET: i32    = 30;
    const ITERATIONS: u64 = 50_000;
    const EXPECTED: i64  = 832_040;

    let mut total: i64 = 0;
    let start = Instant::now();

    for _ in 0..ITERATIONS {
        total += fib_tco(TARGET);
    }

    let elapsed  = start.elapsed();
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us   = (elapsed.as_micros() as f64) / ITERATIONS as f64;

    println!("--- Native Rust Benchmark (matches FASM fibonacci.fasm) ---");
    println!("Algorithm : iterative accumulator (mirrors FASM TAIL_CALL)");
    println!("N         : {}", TARGET);
    println!("Iterations: {}", ITERATIONS);
    println!("Total time: {:.2} ms", total_ms);
    println!("Time/iter : {:.4} us  <-- compare with `fasm bench`", avg_us);
    println!("Result    : {} per iter (sanity check: {})",
             total / (ITERATIONS as i64),
             EXPECTED * (ITERATIONS as i64));
}
