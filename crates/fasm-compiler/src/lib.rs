pub mod token;
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod validator;
pub mod emitter;

pub use emitter::compile;

/// Compile FASM source text directly into an in-memory Program.
pub fn compile_source(source: &str) -> Result<fasm_bytecode::Program, String> {
    let tokens = lexer::tokenize(source)?;
    let ast    = parser::parse(tokens)?;
    validator::validate(&ast)?;
    emitter::emit(ast)
}
