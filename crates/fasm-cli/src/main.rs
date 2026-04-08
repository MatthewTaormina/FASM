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
        _ => {
            eprintln!("Unknown command '{}'. Use compile, run, check, or exec.", command);
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

    match sandbox.run(&program) {
        Ok(_) => {}
        Err(e) => { eprintln!("Runtime error: {}", e); std::process::exit(1); }
    }
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

fn print_usage() {
    eprintln!("FASM Virtual Machine");
    eprintln!("Usage:");
    eprintln!("  fasm compile <file.fasm> [-o output.fasmc]  Compile to bytecode");
    eprintln!("  fasm run     <file.fasm> [--clock-hz N]     Compile and run");
    eprintln!("  fasm exec    <file.fasmc> [--clock-hz N]    Run pre-compiled bytecode");
    eprintln!("  fasm check   <file.fasm>                    Validate only");
}
