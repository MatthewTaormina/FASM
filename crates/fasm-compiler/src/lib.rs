//! # fasm-compiler
//!
//! Translates FASM source text into a [`fasm_bytecode::Program`] ready for the VM.
//!
//! ## Pipeline
//!
//! ```text
//! Source text
//!   ↓ lexer::tokenize()
//! Vec<Token>
//!   ↓ parser::parse()
//! ProgramAst
//!   ↓ validator::validate()
//! (validated AST)
//!   ↓ emitter::emit()
//! Program (bytecode)
//! ```
//!
//! The quickest entry point is [`compile_source`], which runs all four stages.
//!
//! ## Stages
//! - [`lexer`] — tokenises FASM source (integers, hex, floats, strings, identifiers, comments)
//! - [`parser`] — builds the AST: directives, functions, params, locals, labels, instructions
//! - [`validator`] — static checks: duplicate names, undefined labels/functions, `Main` presence
//! - [`emitter`] — two-pass code generation with `DEFINE` constant resolution

pub mod ast;
pub mod emitter;
pub mod lexer;
pub mod parser;
pub mod token;
pub mod validator;

pub use emitter::compile;

/// Compile FASM source text directly into an in-memory Program.
pub fn compile_source(source: &str) -> Result<fasm_bytecode::Program, String> {
    let tokens = lexer::tokenize(source)?;
    let ast = parser::parse(tokens)?;
    validator::validate(&ast)?;
    emitter::emit(ast)
}
