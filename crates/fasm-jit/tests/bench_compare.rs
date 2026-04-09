use fasm_compiler::compile_source;
use fasm_jit::{analyze_program, compile_program, codegen};
use fasm_vm::{value::FasmStruct, Value, Executor};
use std::time::Instant;

const ITERS: u32 = 50_000;

fn fib_src() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/fasm-engine/tests/fixtures/fib_handler.fasm"
    )).unwrap()
}

#[test]
fn compare_jit_vs_interpreter() {
    let src = fib_src();
    let program = compile_source(&src).expect("compile");

    // Interpreter timing
    let t0 = Instant::now();
    for _ in 0..ITERS {
        let mut ex = Executor::new();
        ex.run(&program).unwrap();
    }
    let interp_ns = t0.elapsed().as_nanos() / ITERS as u128;

    // JIT timing
    let eligible = analyze_program(&program);
    let cache = compile_program(&program, &eligible).expect("jit compile");
    let fib_idx = program.get_function_index("FibHandler").unwrap();
    let entry = cache.entries.get(&fib_idx).expect("jit entry");
    let args = Value::Struct(FasmStruct::default());

    let t1 = Instant::now();
    for _ in 0..ITERS {
        unsafe { codegen::call_jit(entry, &args) };
    }
    let jit_ns = t1.elapsed().as_nanos() / ITERS as u128;

    let speedup = interp_ns as f64 / jit_ns as f64;
    println!("\n=== JIT vs Interpreter (FibHandler, fib(30)) ===");
    println!("  Interpreter: {} ns/iter", interp_ns);
    println!("  JIT:         {} ns/iter", jit_ns);
    println!("  Speedup:     {:.2}x", speedup);

    // JIT should not regress vs interpreter
    assert!(speedup > 0.5, "JIT is more than 2x slower than interpreter");
}
