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
