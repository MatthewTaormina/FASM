// Debug tool: prints the instructions in a function
use std::fs;
use fasm_compiler::compile_source;

fn main() {
    let source = fs::read_to_string("examples/fibonacci.fasm").unwrap();
    let program = compile_source(&source).unwrap();

    for func in &program.functions {
        println!("=== {} ({} instructions) ===", func.name, func.instructions.len());
        for (i, instr) in func.instructions.iter().enumerate() {
            println!("  {:3}: {:?}", i, instr);
        }
        println!();
    }
}
