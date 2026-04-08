//! # fasm-bytecode
//!
//! Instruction model, opcode definitions, type tags, and binary encode/decode
//! for the FASM virtual machine file format (`.fasmc`).
//!
//! ## File Format
//! Encoded programs begin with the magic bytes `FSMC` followed by a version byte.
//! Instructions use a variable-width encoding: `[opcode u8][operand_count u8][operands...]`.
//!
//! ## Key Types
//! - [`Opcode`] — all VM opcodes as a `u8`-backed enum
//! - [`FasmType`] — all primitive, collection, and wrapper type tags
//! - [`Instruction`] — a decoded instruction with opcode and operand list
//! - [`Operand`] — a single operand (slot reference, immediate, label target, etc.)
//! - [`Program`] — a fully compiled FASM program ready for execution
//! - [`encode_program`] / [`decode_program`] — binary serialisation round-trip

pub mod opcode;
pub mod types;
pub mod instruction;
pub mod program;
pub mod encode;

pub use opcode::Opcode;
pub use types::FasmType;
pub use instruction::{Instruction, Operand};
pub use program::{Program, FunctionDef, ParamDescriptor};
pub use encode::{encode_program, decode_program};
