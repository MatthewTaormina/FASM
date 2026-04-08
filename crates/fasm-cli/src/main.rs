use std::{env, fs, path::Path};
use fasm_bytecode::{encode_program, decode_program};
use fasm_compiler::compile_source;
use fasm_sandbox::Sandbox;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        print_usage();
        std::process::exit(1);
    }

    let command = args[1].as_str();
    let file    = &args[2];

    match command {
        "compile" => cmd_compile(file, &args[3..]),
        "run"     => cmd_run(file, &args[3..]),
        "check"   => cmd_check(file),
        "exec"    => cmd_exec(file, &args[3..]),
        "bench"   => cmd_bench(file, &args[3..]),
        _ => {
            eprintln!("Unknown command '{}'. Use compile, run, check, exec, or bench.", command);
            print_usage();
            std::process::exit(1);
        }
    }
}

/// fasm compile <file.fasm> [-o output.fasmc]
fn cmd_compile(file: &str, extra: &[String]) {
    let source = read_file(file);
    let program = match compile_source(&source) {
        Ok(p) => p,
        Err(e) => { eprintln!("Compile error: {}", e); std::process::exit(1); }
    };

    let out_path = extra.windows(2)
        .find(|w| w[0] == "-o")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| {
            Path::new(file).with_extension("fasmc").to_string_lossy().into_owned()
        });

    let bytes = encode_program(&program);
    fs::write(&out_path, &bytes).expect("Failed to write output file");
    println!("Compiled {} -> {} ({} bytes)", file, out_path, bytes.len());
}

/// fasm run <file.fasm> [--clock-hz N]
fn cmd_run(file: &str, extra: &[String]) {
    let source = read_file(file);
    let program = match compile_source(&source) {
        Ok(p) => p,
        Err(e) => { eprintln!("Compile error: {}", e); std::process::exit(1); }
    };

    let clock_hz = parse_clock_hz(extra);
    let mut sandbox = Sandbox::new(0);
    if let Some(hz) = clock_hz { sandbox.set_clock_hz(hz); }
    parse_plugins(extra, &mut sandbox);

    match sandbox.run(&program) {
        Ok(_) => {}
        Err(e) => { eprintln!("Runtime error: {}", e); std::process::exit(1); }
    }
}

/// fasm check <file.fasm> — validate only, no execution
fn cmd_check(file: &str) {
    let source = read_file(file);
    match compile_source(&source) {
        Ok(prog) => {
            println!("OK — {} function(s), {} global init(s)",
                prog.functions.len(), prog.global_inits.len());
        }
        Err(e) => {
            eprintln!("Validation error: {}", e);
            std::process::exit(1);
        }
    }
}

/// fasm exec <file.fasmc> — run pre-compiled bytecode
fn cmd_exec(file: &str, extra: &[String]) {
    let bytes = fs::read(file).unwrap_or_else(|_| {
        eprintln!("Cannot read file '{}'", file);
        std::process::exit(1);
    });
    let program = match decode_program(&bytes) {
        Ok(p) => p,
        Err(e) => { eprintln!("Decode error: {}", e); std::process::exit(1); }
    };

    let clock_hz = parse_clock_hz(extra);
    let mut sandbox = Sandbox::new(0);
    if let Some(hz) = clock_hz { sandbox.set_clock_hz(hz); }
    parse_plugins(extra, &mut sandbox);

    match sandbox.run(&program) {
        Ok(_) => {}
        Err(e) => { eprintln!("Runtime error: {}", e); std::process::exit(1); }
    }
}

/// fasm bench <file.fasmc> [iterations] — benchmark VM execution on pre-loaded AST.
fn cmd_bench(file: &str, extra: &[String]) {
    let bytes = fs::read(file).unwrap_or_else(|_| {
        eprintln!("Cannot read file '{}'", file);
        std::process::exit(1);
    });
    let program = match decode_program(&bytes) {
        Ok(p) => p,
        Err(e) => { eprintln!("Decode error: {}", e); std::process::exit(1); }
    };

    let iterations: usize = extra.get(0).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    println!("Benchmarking '{}' with pre-loaded VM across {} iterations...", file, iterations);

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        // We evaluate an entirely fresh Sandbox isolation wrapper per-run,
        // measuring exactly the cost of a FaaS boundary injection + runtime.
        let mut sandbox = Sandbox::new(0);
        
        // Suppress stdout by hijacking syscall 0/1 to do nothing? 
        // For a pure execution bench, we let it run as-is. Better point it at non-I/O scripts.
        if let Err(e) = sandbox.run(&program) {
            eprintln!("Runtime error during benchmark: {}", e);
            std::process::exit(1);
        }
    }
    let elapsed = start.elapsed();
    
    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let avg_us = (elapsed.as_micros() as f64) / (iterations as f64);
    println!("--- Benchmark complete ---");
    println!("Total time: {:.2} ms", total_ms);
    println!("Time per execution: {:.2} μs (microseconds)", avg_us);
}

fn read_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| {
        eprintln!("Cannot read file '{}'", path);
        std::process::exit(1);
    })
}

fn parse_clock_hz(args: &[String]) -> Option<u64> {
    args.windows(2)
        .find(|w| w[0] == "--clock-hz")
        .and_then(|w| w[1].parse::<u64>().ok())
}

fn parse_plugins(args: &[String], sandbox: &mut Sandbox) {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--plugin" {
            if let Some(val) = iter.next() {
                // format: ID:CMD:ARGS...
                let parts: Vec<&str> = val.split(':').collect();
                if parts.len() >= 2 {
                    if let Ok(id) = parts[0].parse::<i32>() {
                        let cmd = parts[1];
                        let cmd_args = &parts[2..];
                        sandbox.mount_sidecar(id, cmd, cmd_args);
                    } else {
                        eprintln!("Invalid plugin ID: {}", parts[0]);
                    }
                } else {
                    eprintln!("Invalid plugin format. Use --plugin ID:CMD:ARG1:ARG2");
                }
            }
        }
    }
}

fn print_usage() {
    eprintln!("FASM Virtual Machine");
    eprintln!("Usage:");
    eprintln!("  fasm compile <file.fasm> [-o output.fasmc]  Compile to bytecode");
    eprintln!("  fasm run     <file.fasm> [--clock-hz N]     Compile and run");
    eprintln!("  fasm exec    <file.fasmc> [--clock-hz N]    Run pre-compiled bytecode");
    eprintln!("  fasm check   <file.fasm>                    Validate only");
}
