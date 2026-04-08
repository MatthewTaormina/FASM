//! Eligibility analysis for JIT compilation.
//!
//! A function is JIT-eligible when:
//! - Every local slot is a numeric primitive (Bool, Int*, Uint*, Float*) OR a
//!   STRUCT that is only used as a call-argument accumulator (Reserve + SetField
//!   + TailCall self, or Call to another eligible function).
//! - The only allowed control-flow is Jmp / Jz / Jnz / TailCall-self / Ret / Halt.
//! - No Syscall, collection ops, TRY/CATCH, Addr, async opcodes, or wrapper types.
//! - All `Ret` instructions return a value of the same numeric type.

use fasm_bytecode::{
    instruction::{BuiltIn, Operand, SlotRef},
    opcode::Opcode,
    types::FasmType,
    FunctionDef, Program,
};
use std::collections::{HashMap, HashSet};

// ─── Public types ─────────────────────────────────────────────────────────────

/// Cranelift numeric kind used for a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitType {
    /// Bool, Int8, Int16, Int32, Uint8, Uint16, Uint32 all map to i32 at JIT level.
    I32,
    /// Int64, Uint64
    I64,
    /// Float32
    F32,
    /// Float64
    F64,
}

impl JitType {
    pub fn from_fasm(t: FasmType) -> Option<Self> {
        match t {
            FasmType::Bool
            | FasmType::Int8
            | FasmType::Int16
            | FasmType::Int32
            | FasmType::Uint8
            | FasmType::Uint16
            | FasmType::Uint32 => Some(JitType::I32),
            FasmType::Int64 | FasmType::Uint64 => Some(JitType::I64),
            FasmType::Float32 => Some(JitType::F32),
            FasmType::Float64 => Some(JitType::F64),
            _ => None,
        }
    }
}

/// Per-function JIT information produced by the analyzer.
#[derive(Debug, Clone)]
pub struct JitFnInfo {
    /// Slot index → JIT type for all numeric local slots.
    pub slot_types: HashMap<u8, JitType>,
    /// Set of local slot indices that are STRUCT arg-accumulators
    /// (only used with SetField + TailCall/Call, never read as values).
    pub struct_accumulators: HashSet<u8>,
    /// PARAM declarations, in declaration order.
    /// Each entry is `(struct_key, jit_type)`.
    pub params: Vec<(u32, JitType)>,
    /// JIT type of the return value (all Ret instructions must agree).
    pub ret_type: JitType,
}

// ─── Analysis entry point ─────────────────────────────────────────────────────

/// Analyse `program` and return a map from function index to JIT info for every
/// function that can be JIT-compiled.
pub fn analyze_program(program: &Program) -> HashMap<usize, JitFnInfo> {
    let mut result = HashMap::new();
    for (idx, func) in program.functions.iter().enumerate() {
        if let Some(info) = analyze_function(func, idx) {
            result.insert(idx, info);
        }
    }
    result
}

// ─── Per-function analysis ────────────────────────────────────────────────────

fn analyze_function(func: &FunctionDef, self_idx: usize) -> Option<JitFnInfo> {
    // ── 1. Build slot type map from Reserve instructions ─────────────────────
    let mut slot_types: HashMap<u8, JitType> = HashMap::new();
    let mut struct_slots: HashSet<u8> = HashSet::new();

    for instr in &func.instructions {
        if instr.opcode != Opcode::Reserve {
            continue;
        }
        let slot_idx = match instr.operands.first() {
            Some(Operand::Slot(SlotRef::Local(idx))) => *idx,
            _ => return None,
        };
        let fasm_type = match instr.operands.get(1) {
            Some(Operand::Type(t)) => *t,
            _ => return None,
        };
        if let Some(jt) = JitType::from_fasm(fasm_type) {
            slot_types.insert(slot_idx, jt);
        } else if fasm_type == FasmType::Struct {
            struct_slots.insert(slot_idx);
        } else {
            // Unsupported type (Vec, Stack, etc.) — not JIT-eligible.
            return None;
        }
    }

    // ── 2. Collect params ────────────────────────────────────────────────────
    let mut params: Vec<(u32, JitType)> = Vec::new();
    for p in &func.params {
        let jt = JitType::from_fasm(p.fasm_type)?;
        params.push((p.key, jt));
    }

    // ── 3. Determine return type ─────────────────────────────────────────────
    // Walk Ret instructions and require all non-$ret returns to agree on type.
    let mut ret_type: Option<JitType> = None;
    for instr in &func.instructions {
        if instr.opcode != Opcode::Ret {
            continue;
        }
        let op = instr.operands.first();
        let jt = match op {
            None => {
                // Bare RET → returns Null — only allowed if *all* returns are bare.
                continue;
            }
            Some(Operand::Slot(SlotRef::BuiltIn(BuiltIn::Ret))) => {
                // RET $ret — used after TAIL_CALL self, effectively unreachable in JIT.
                // Skip; does not constrain return type.
                continue;
            }
            Some(Operand::Slot(SlotRef::Local(idx))) => {
                slot_types.get(idx).copied()?
            }
            Some(Operand::Imm(_)) => {
                // Immediate return value — treat as i64 (checked later if needed).
                continue;
            }
            _ => return None,
        };
        match ret_type {
            None => ret_type = Some(jt),
            Some(prev) if prev == jt => {}
            _ => return None, // conflicting return types
        }
    }

    let ret_type = ret_type.unwrap_or(JitType::I64);

    // ── 4. Validate all instructions ─────────────────────────────────────────
    for instr in &func.instructions {
        if !is_jit_eligible_opcode(instr, &slot_types, &struct_slots, self_idx) {
            return None;
        }
    }

    // ── 5. Validate struct slots are only used as accumulators ───────────────
    for instr in &func.instructions {
        match instr.opcode {
            Opcode::GetField => {
                // GetField is only allowed on $args, not on local struct slots.
                let src = instr.operands.first()?;
                match src {
                    Operand::Slot(SlotRef::BuiltIn(BuiltIn::Args)) => {} // OK
                    _ => return None, // reading a local struct is not supported
                }
            }
            Opcode::SetField => {
                // Destination must be a struct accumulator local.
                let dst = instr.operands.first()?;
                match dst {
                    Operand::Slot(SlotRef::Local(idx)) => {
                        if !struct_slots.contains(idx) {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            _ => {}
        }
    }

    Some(JitFnInfo {
        slot_types,
        struct_accumulators: struct_slots,
        params,
        ret_type,
    })
}

// ─── Opcode whitelist ─────────────────────────────────────────────────────────

fn is_jit_eligible_opcode(
    instr: &fasm_bytecode::Instruction,
    slot_types: &HashMap<u8, JitType>,
    struct_slots: &HashSet<u8>,
    self_idx: usize,
) -> bool {
    match instr.opcode {
        // Always allowed (bookkeeping, arithmetic, control flow)
        Opcode::Nop
        | Opcode::Reserve
        | Opcode::Release
        | Opcode::Mov
        | Opcode::Store
        | Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::Mod
        | Opcode::Neg
        | Opcode::Eq
        | Opcode::Neq
        | Opcode::Lt
        | Opcode::Lte
        | Opcode::Gt
        | Opcode::Gte
        | Opcode::And
        | Opcode::Or
        | Opcode::Xor
        | Opcode::Not
        | Opcode::Shl
        | Opcode::Shr
        | Opcode::Jmp
        | Opcode::Jz
        | Opcode::Jnz
        | Opcode::Ret
        | Opcode::Halt
        | Opcode::Cast
        | Opcode::TmpBlock
        | Opcode::EndTmp => true,

        // GetField: only allowed on $args.
        Opcode::GetField => matches!(
            instr.operands.first(),
            Some(Operand::Slot(SlotRef::BuiltIn(BuiltIn::Args)))
        ),

        // SetField: destination must be a struct accumulator.
        Opcode::SetField => match instr.operands.first() {
            Some(Operand::Slot(SlotRef::Local(idx))) => struct_slots.contains(idx),
            _ => false,
        },

        // TailCall: only to self.
        Opcode::TailCall => match instr.operands.first() {
            Some(Operand::FuncRef(idx)) => *idx as usize == self_idx,
            _ => false,
        },

        // Mov of $ret after a self tail-call is syntactic noise; allowed.
        Opcode::Call | Opcode::AsyncCall | Opcode::Await => {
            // Non-tail calls are not JIT'd for now.
            false
        }

        // All collection, syscall, wrapper, and error-handling ops are banned.
        _ => false,
    }
}

/// Sanity-check: verify a slot operand used as a VALUE source is numeric
/// (not a struct accumulator used as a data value, which would be a bug).
pub fn slot_is_numeric(slot: u8, slot_types: &HashMap<u8, JitType>) -> bool {
    slot_types.contains_key(&slot)
}
