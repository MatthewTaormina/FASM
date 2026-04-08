use crate::types::FasmType;

/// A slot reference — either a local frame index or a global register index.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotRef {
    Local(u8),
    Global(u16),
    /// Tmp block dynamic register.
    Tmp(u8),
    /// Dereferenced (&slot) — follow the reference stored in this slot.
    DerefLocal(u8),
    DerefGlobal(u16),
    DerefTmp(u8),
    /// Special built-in symbol: $args (incoming struct), $ret (return value),
    /// $fault_index, $fault_code.
    BuiltIn(BuiltIn),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltIn {
    Args,
    Ret,
    FaultIndex,
    FaultCode,
}

/// A single operand inside an instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Slot(SlotRef),
    /// Inline immediate value.
    Imm(Immediate),
    /// Index into the program's string/function-name table.
    FuncRef(u16),
    /// Absolute instruction index target (resolved from labels by emitter).
    LabelTarget(u32),
    /// Syscall id.
    SyscallId(i32),
    /// Type tag for RESERVE / CAST.
    Type(FasmType),
    /// STRUCT field key (u32).
    Key(u32),
    /// Required / optional flag for PARAM (true = required).
    Required(bool),
}

/// Immediate scalar values embeddable in instructions.
#[derive(Debug, Clone, PartialEq)]
pub enum Immediate {
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    Uint8(u8),
    Uint16(u16),
    Uint32(u32),
    Uint64(u64),
    Float32(f32),
    Float64(f64),
    Null,
    /// A UTF-8 string literal. The VM expands this into a `VEC<UINT8>` at runtime.
    Str(String),
}

/// A decoded FASM instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    pub opcode: crate::Opcode,
    pub operands: Vec<Operand>,
}

impl Instruction {
    pub fn new(opcode: crate::Opcode, operands: Vec<Operand>) -> Self {
        Self { opcode, operands }
    }

    pub fn no_args(opcode: crate::Opcode) -> Self {
        Self {
            opcode,
            operands: vec![],
        }
    }
}
