//! Cranelift-based JIT compiler for eligible FASM functions.
//!
//! Compiles a [`FunctionDef`] described by a [`JitFnInfo`] to native machine
//! code via the Cranelift JIT backend.  Each compiled function uses the
//! native C ABI with parameter widths matching the declared FASM types.

use crate::analyze::{JitFnInfo, JitType};
use cranelift_codegen::ir::{
    condcodes::IntCC,
    types::{F32, F64, I32, I64},
    AbiParam, Block, BlockArg, Function, InstBuilder, Signature, Type, Value as CVal,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use fasm_bytecode::{
    instruction::{BuiltIn, Immediate, Operand, SlotRef},
    opcode::Opcode,
    FunctionDef, Program,
};
use fasm_vm::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

// ─── Public API ──────────────────────────────────────────────────────────────

/// A compiled JIT function entry.
pub struct JitEntry {
    /// Raw function pointer (C ABI).
    /// Signature depends on `params.len()` and each param's `JitType`.
    pub fn_ptr: *const u8,
    /// Parameters in declaration order: `(struct_key, jit_type)`.
    pub params: Vec<(u32, JitType)>,
    /// Return JIT type.
    pub ret_type: JitType,
}

// SAFETY: the compiled memory is owned by `JitCache._module` and is
// read-only executable — safe to share across threads.
unsafe impl Send for JitEntry {}
unsafe impl Sync for JitEntry {}

/// Cache of all JIT-compiled functions for a loaded program.
pub struct JitCache {
    /// Map from function index → compiled entry.
    pub entries: HashMap<usize, JitEntry>,
    /// Must stay alive to keep the mapped executable pages valid.
    _module: JITModule,
}

// SAFETY: after `finalize_definitions()` the JITModule only holds read-only
// executable pages.  We never call mutating methods on it after construction.
unsafe impl Send for JitCache {}
unsafe impl Sync for JitCache {}

// ─── Compilation entry point ──────────────────────────────────────────────────

/// Compile all eligible functions in `program`.
/// Returns `None` if the host ISA is unsupported (very rare).
pub fn compile_program(
    program: &Program,
    eligible: &HashMap<usize, JitFnInfo>,
) -> Option<JitCache> {
    if eligible.is_empty() {
        return None;
    }

    let builder = JITBuilder::new(cranelift_module::default_libcall_names()).ok()?;
    let mut module = JITModule::new(builder);

    // Pass 1 — declare function IDs.
    let mut func_ids: HashMap<usize, cranelift_module::FuncId> = HashMap::new();
    for (&func_idx, info) in eligible {
        let sig = build_signature(&module, info);
        let name = format!("__fasm_jit_{}", func_idx);
        let id = module
            .declare_function(&name, Linkage::Export, &sig)
            .ok()?;
        func_ids.insert(func_idx, id);
    }

    // Pass 2 — compile each function body.
    let mut fn_builder_ctx = FunctionBuilderContext::new();
    for (&func_idx, info) in eligible {
        let func_def = &program.functions[func_idx];
        let sig = build_signature(&module, info);
        let mut ctx = module.make_context();
        ctx.func.signature = sig;

        if compile_fn(
            &mut ctx.func,
            &mut fn_builder_ctx,
            func_def,
            info,
            func_idx,
        ).is_none() {
            eprintln!("[fasm-jit] compile_fn returned None for func_idx={}", func_idx);
            return None;
        }

        let func_id = func_ids[&func_idx];
        if let Err(e) = module.define_function(func_id, &mut ctx) {
            eprintln!("[fasm-jit] define_function error for func_idx={}: {:?}", func_idx, e);
            return None;
        }
        module.clear_context(&mut ctx);
    }

    if let Err(e) = module.finalize_definitions() {
        eprintln!("[fasm-jit] finalize_definitions error: {:?}", e);
        return None;
    }

    // Pass 3 — gather function pointers.
    let mut entries = HashMap::new();
    for (&func_idx, &func_id) in &func_ids {
        let fn_ptr = module.get_finalized_function(func_id) as *const u8;
        let info = &eligible[&func_idx];
        entries.insert(
            func_idx,
            JitEntry {
                fn_ptr,
                params: info.params.clone(),
                ret_type: info.ret_type,
            },
        );
    }

    Some(JitCache {
        entries,
        _module: module,
    })
}

// ─── Signature helpers ────────────────────────────────────────────────────────

fn jit_ir_type(jt: JitType) -> Type {
    match jt {
        JitType::I32 => I32,
        JitType::I64 => I64,
        JitType::F32 => F32,
        JitType::F64 => F64,
    }
}

fn build_signature(module: &JITModule, info: &JitFnInfo) -> Signature {
    let mut sig = module.make_signature();
    for &(_key, jt) in &info.params {
        sig.params.push(AbiParam::new(jit_ir_type(jt)));
    }
    sig.returns.push(AbiParam::new(jit_ir_type(info.ret_type)));
    sig
}

// ─── Per-function codegen ─────────────────────────────────────────────────────

/// Compile one FASM function body into `func`.
///
/// ## Block layout
/// Cranelift prohibits back-edges to the function entry block.  We therefore
/// emit two preamble blocks:
///
/// ```text
/// entry_blk  (ABI entry — has function params, no back-edges)
///   ↓ jump
/// loop_blk   (loop header — carries looping Variables, target of TailCall)
///   ↓ … all regular FASM basic blocks …
/// ```
///
/// `GetField $args, key, slot` reads from Cranelift Variables that are
/// initialised in `entry_blk` from the ABI params and updated on each
/// TailCall back-edge.  This way the variables act as φ-nodes at `loop_blk`.
fn compile_fn(
    func: &mut Function,
    fn_builder_ctx: &mut FunctionBuilderContext,
    func_def: &FunctionDef,
    info: &JitFnInfo,
    self_idx: usize,
) -> Option<()> {
    let instrs = &func_def.instructions;

    // ── Collect basic block boundaries ────────────────────────────────────
    let mut block_starts: BTreeSet<usize> = BTreeSet::new();
    block_starts.insert(0);
    for (ip, instr) in instrs.iter().enumerate() {
        match instr.opcode {
            Opcode::Jmp | Opcode::Jz | Opcode::Jnz => {
                if let Some(Operand::LabelTarget(t)) = instr.operands.last() {
                    block_starts.insert(*t as usize);
                }
                block_starts.insert(ip + 1);
            }
            Opcode::TailCall | Opcode::Ret | Opcode::Halt => {
                block_starts.insert(ip + 1);
            }
            _ => {}
        }
    }

    // ── Create Cranelift blocks ────────────────────────────────────────────
    let mut builder = FunctionBuilder::new(func, fn_builder_ctx);

    // entry_blk: one-time ABI entry — never jumped back to.
    let entry_blk = builder.create_block();
    builder.append_block_params_for_function_params(entry_blk);

    // loop_blk: the real loop header (ip=0 maps here, TailCall jumps here).
    let loop_blk = builder.create_block();

    // Map all FASM IPs → Cranelift blocks.  IP 0 → loop_blk.
    let mut ip_to_block: HashMap<usize, Block> = HashMap::new();
    ip_to_block.insert(0, loop_blk);
    for &start in &block_starts {
        if start == 0 {
            continue;
        }
        ip_to_block.insert(start, builder.create_block());
    }

    // ── Declare Cranelift Variables ───────────────────────────────────────
    let sorted_slots: BTreeMap<u8, JitType> =
        info.slot_types.iter().map(|(&k, &v)| (k, v)).collect();

    // One Variable per numeric local slot.
    let mut vars: HashMap<u8, Variable> = HashMap::new();
    for (&slot, &jt) in &sorted_slots {
        let v = builder.declare_var(jit_ir_type(jt));
        vars.insert(slot, v);
    }

    // One Variable per PARAM key — these are the loop-carried arguments.
    // They are initialised in entry_blk from ABI params and updated by
    // TailCall before the back-edge jump to loop_blk.
    let mut param_vars: HashMap<u32, Variable> = HashMap::new();
    for &(key, jt) in &info.params {
        let v = builder.declare_var(jit_ir_type(jt));
        param_vars.insert(key, v);
    }

    let ret_var = builder.declare_var(jit_ir_type(info.ret_type));

    // ── Emit prologue in entry_blk ────────────────────────────────────────
    builder.switch_to_block(entry_blk);
    builder.seal_block(entry_blk); // no predecessors

    // Initialise param_vars from ABI params.
    for (i, &(key, _jt)) in info.params.iter().enumerate() {
        let param_val = builder.block_params(entry_blk)[i];
        builder.def_var(param_vars[&key], param_val);
    }

    // Initialise all local vars + ret_var to zero.
    for (&slot, &jt) in &sorted_slots {
        let z = zero_val(&mut builder, jt);
        builder.def_var(vars[&slot], z);
    }
    {
        let z = zero_val(&mut builder, info.ret_type);
        builder.def_var(ret_var, z);
    }

    // Fall through to loop_blk.
    {
        let no_args: Vec<BlockArg> = Vec::new();
        builder.ins().jump(loop_blk, no_args.iter());
    }

    // ── Emit FASM instruction body starting at loop_blk ───────────────────
    // `setfield_vals[(struct_slot, key)] = CVal` — collects TailCall args.
    let mut setfield_vals: HashMap<(u8, u32), CVal> = HashMap::new();

    let mut ip: usize = 0;
    let mut current_blk = loop_blk;
    let mut terminated = false;

    builder.switch_to_block(loop_blk);

    while ip <= instrs.len() {
        if let Some(&blk) = ip_to_block.get(&ip) {
            if blk != current_blk {
                if !terminated {
                    let no_args: Vec<BlockArg> = Vec::new();
                    builder.ins().jump(blk, no_args.iter());
                }
                builder.switch_to_block(blk);
                terminated = false;
                current_blk = blk;
            }
        }

        if ip == instrs.len() {
            break;
        }

        let instr = &instrs[ip];
        let cur_ip = ip;
        ip += 1;

        let step: Option<()> = (|| { match instr.opcode {
            // ── No-ops ──────────────────────────────────────────────────────
            Opcode::Nop | Opcode::TmpBlock | Opcode::EndTmp | Opcode::Release => {}

            Opcode::Reserve => {
                // No action: numeric slots are zero-initialised in prologue;
                // struct accumulator slots are tracked via setfield_vals.
            }

            // ── Data movement ────────────────────────────────────────────────
            Opcode::Mov => {
                let val = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                write_op(&mut builder, instr.operands.get(1)?, val, &vars, &ret_var)?;
            }
            Opcode::Store => {
                let val = imm_to_cv(&mut builder, instr.operands.first()?, info.ret_type)?;
                write_op(&mut builder, instr.operands.get(1)?, val, &vars, &ret_var)?;
            }

            // ── Arithmetic ───────────────────────────────────────────────────
            Opcode::Add => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Add)?,
            Opcode::Sub => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Sub)?,
            Opcode::Mul => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Mul)?,
            Opcode::Div => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Div)?,
            Opcode::Mod => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Mod)?,
            Opcode::And => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::And)?,
            Opcode::Or  => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Or)?,
            Opcode::Xor => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Xor)?,
            Opcode::Shl => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Shl)?,
            Opcode::Shr => emit_binop(&mut builder, instr, &vars, &ret_var, info.ret_type, BinOp::Shr)?,

            Opcode::Neg => {
                let a = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                let ty = builder.func.dfg.value_type(a);
                let result = if ty == F32 || ty == F64 {
                    builder.ins().fneg(a)
                } else {
                    let zero = builder.ins().iconst(ty, 0);
                    builder.ins().isub(zero, a)
                };
                write_op(&mut builder, instr.operands.get(1)?, result, &vars, &ret_var)?;
            }
            Opcode::Not => {
                let a = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                let ty = builder.func.dfg.value_type(a);
                let ones = builder.ins().iconst(ty, -1i64);
                let result = builder.ins().bxor(a, ones);
                write_op(&mut builder, instr.operands.get(1)?, result, &vars, &ret_var)?;
            }

            // ── Comparison ───────────────────────────────────────────────────
            Opcode::Eq  => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::Equal)?,
            Opcode::Neq => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::NotEqual)?,
            Opcode::Lt  => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::SignedLessThan)?,
            Opcode::Lte => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::SignedLessThanOrEqual)?,
            Opcode::Gt  => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::SignedGreaterThan)?,
            Opcode::Gte => emit_cmp(&mut builder, instr, &vars, &ret_var, info.ret_type, IntCC::SignedGreaterThanOrEqual)?,

            // ── Control flow ─────────────────────────────────────────────────
            Opcode::Jmp => {
                let target_ip = label_target_ip(instr.operands.last()?)?;
                let target_blk = *ip_to_block.get(&target_ip)?;
                let no_args: Vec<BlockArg> = Vec::new();
                builder.ins().jump(target_blk, no_args.iter());
                terminated = true;
            }
            Opcode::Jnz => {
                let cond = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                let target_ip = label_target_ip(instr.operands.get(1)?)?;
                let target_blk = *ip_to_block.get(&target_ip)?;
                let fall_blk   = *ip_to_block.get(&ip)?;
                let nz = is_truthy(&mut builder, cond);
                let t_args: Vec<BlockArg> = Vec::new();
                let f_args: Vec<BlockArg> = Vec::new();
                builder.ins().brif(nz, target_blk, t_args.iter(), fall_blk, f_args.iter());
                terminated = true;
            }
            Opcode::Jz => {
                let cond = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                let target_ip = label_target_ip(instr.operands.get(1)?)?;
                let target_blk = *ip_to_block.get(&target_ip)?;
                let fall_blk   = *ip_to_block.get(&ip)?;
                let z = is_zero(&mut builder, cond);
                let t_args: Vec<BlockArg> = Vec::new();
                let f_args: Vec<BlockArg> = Vec::new();
                builder.ins().brif(z, target_blk, t_args.iter(), fall_blk, f_args.iter());
                terminated = true;
            }

            // ── Struct field access ───────────────────────────────────────────
            Opcode::GetField => {
                // Only $args is supported: reads from the loop-carried param Variable.
                match instr.operands.first() {
                    Some(Operand::Slot(SlotRef::BuiltIn(BuiltIn::Args))) => {}
                    _ => return None,
                }
                let key = match instr.operands.get(1) {
                    Some(Operand::Key(k)) => *k,
                    _ => return None,
                };
                let pv = *param_vars.get(&key)?;
                let param_val = builder.use_var(pv);
                let dst_op = instr.operands.get(2)?;
                write_op(&mut builder, dst_op, param_val, &vars, &ret_var)?;
            }
            Opcode::SetField => {
                // Accumulate (struct_slot, key) → CVal for the next TailCall.
                let struct_slot = match instr.operands.first() {
                    Some(Operand::Slot(SlotRef::Local(s))) => *s,
                    _ => return None,
                };
                let key = match instr.operands.get(1) {
                    Some(Operand::Key(k)) => *k,
                    _ => return None,
                };
                let val = read_op(&mut builder, instr.operands.get(2)?, &vars, &ret_var, info.ret_type)?;
                setfield_vals.insert((struct_slot, key), val);
            }

            // ── Self tail-call → loop back to loop_blk ────────────────────────
            Opcode::TailCall => {
                let target_idx = match instr.operands.first() {
                    Some(Operand::FuncRef(idx)) => *idx as usize,
                    _ => return None,
                };
                if target_idx != self_idx {
                    return None;
                }
                let struct_slot = match instr.operands.get(1) {
                    Some(Operand::Slot(SlotRef::Local(s))) => *s,
                    _ => return None,
                };
                // Update param_vars then jump to loop_blk.
                // We must collect all values BEFORE updating any variables to
                // avoid use-after-redefine ordering hazards.
                let mut new_vals: Vec<(u32, CVal)> = Vec::with_capacity(info.params.len());
                for &(param_key, _jt) in &info.params {
                    let val = *setfield_vals.get(&(struct_slot, param_key))?;
                    new_vals.push((param_key, val));
                }
                for (param_key, val) in new_vals {
                    let pv = *param_vars.get(&param_key)?;
                    builder.def_var(pv, val);
                }
                let no_args: Vec<BlockArg> = Vec::new();
                builder.ins().jump(loop_blk, no_args.iter());
                terminated = true;
            }

            // ── Return ────────────────────────────────────────────────────────
            Opcode::Ret => {
                let op = instr.operands.first();
                let ret_val = match op {
                    None => zero_val(&mut builder, info.ret_type),
                    Some(Operand::Slot(SlotRef::BuiltIn(BuiltIn::Ret))) => {
                        builder.use_var(ret_var)
                    }
                    Some(op) => {
                        let v = read_op(&mut builder, op, &vars, &ret_var, info.ret_type)?;
                        coerce_to(&mut builder, v, jit_ir_type(info.ret_type))
                    }
                };
                builder.ins().return_(&[ret_val]);
                terminated = true;
            }

            Opcode::Halt => {
                let z = zero_val(&mut builder, info.ret_type);
                builder.ins().return_(&[z]);
                terminated = true;
            }

            Opcode::Cast => {
                let val = read_op(&mut builder, instr.operands.first()?, &vars, &ret_var, info.ret_type)?;
                let target_ft = match instr.operands.get(1) {
                    Some(Operand::Type(t)) => *t,
                    _ => return None,
                };
                let target_jt = JitType::from_fasm(target_ft)?;
                let result = coerce_to(&mut builder, val, jit_ir_type(target_jt));
                write_op(&mut builder, instr.operands.get(2)?, result, &vars, &ret_var)?;
            }

            _ => return None,
        } Some(()) })();

        if step.is_none() {
            eprintln!("[fasm-jit] compile_fn: ip={} opcode={:?} operands={:?} → None",
                cur_ip, instr.opcode, instr.operands);
            return None;
        }
    }

    // Terminate any dangling block (trailing dead block after last Ret).
    if !terminated {
        let z = zero_val(&mut builder, info.ret_type);
        builder.ins().return_(&[z]);
    }

    builder.seal_all_blocks();
    builder.finalize();
    Some(())
}

// ─── Operand read / write ─────────────────────────────────────────────────────

fn read_op(
    builder: &mut FunctionBuilder<'_>,
    op: &Operand,
    vars: &HashMap<u8, Variable>,
    ret_var: &Variable,
    ret_hint: JitType,
) -> Option<CVal> {
    match op {
        Operand::Slot(SlotRef::Local(idx)) => {
            let v = vars.get(idx)?;
            Some(builder.use_var(*v))
        }
        Operand::Slot(SlotRef::BuiltIn(BuiltIn::Ret)) => Some(builder.use_var(*ret_var)),
        Operand::Imm(_) => imm_to_cv(builder, op, ret_hint),
        _ => None,
    }
}

fn write_op(
    builder: &mut FunctionBuilder<'_>,
    op: &Operand,
    val: CVal,
    vars: &HashMap<u8, Variable>,
    ret_var: &Variable,
) -> Option<()> {
    match op {
        Operand::Slot(SlotRef::Local(idx)) => {
            let v = vars.get(idx)?;
            // Coerce to the declared Variable type.
            // We must call use_var first (mut borrow) then dfg.value_type (immutable).
            let current = builder.use_var(*v);
            let var_ty = builder.func.dfg.value_type(current);
            let val = coerce_to(builder, val, var_ty);
            builder.def_var(*v, val);
            Some(())
        }
        Operand::Slot(SlotRef::BuiltIn(BuiltIn::Ret)) => {
            builder.def_var(*ret_var, val);
            Some(())
        }
        _ => None,
    }
}

/// Convert a FASM `Immediate` operand to a Cranelift constant.
fn imm_to_cv(
    builder: &mut FunctionBuilder<'_>,
    op: &Operand,
    hint_ret: JitType,
) -> Option<CVal> {
    let imm = match op {
        Operand::Imm(i) => i,
        _ => return None,
    };
    Some(match imm {
        Immediate::Bool(b)   => builder.ins().iconst(I32, *b as i64),
        Immediate::Int8(n)   => builder.ins().iconst(I32, *n as i64),
        Immediate::Int16(n)  => builder.ins().iconst(I32, *n as i64),
        Immediate::Int32(n)  => builder.ins().iconst(I32, *n as i64),
        Immediate::Int64(n)  => builder.ins().iconst(I64, *n),
        Immediate::Uint8(n)  => builder.ins().iconst(I32, *n as i64),
        Immediate::Uint16(n) => builder.ins().iconst(I32, *n as i64),
        Immediate::Uint32(n) => builder.ins().iconst(I32, *n as i64),
        Immediate::Uint64(n) => builder.ins().iconst(I64, *n as i64),
        Immediate::Float32(f) => builder.ins().f32const(*f),
        Immediate::Float64(f) => builder.ins().f64const(*f),
        Immediate::Null => builder.ins().iconst(jit_ir_type(hint_ret), 0),
        Immediate::Str(_) => return None,
    })
}

// ─── Type coercion ────────────────────────────────────────────────────────────

/// Widen or narrow `val` to `target` IR type.
fn coerce_to(builder: &mut FunctionBuilder<'_>, val: CVal, target: Type) -> CVal {
    let src = builder.func.dfg.value_type(val);
    if src == target {
        return val;
    }
    match (src, target) {
        (I32, I64) => builder.ins().sextend(target, val),
        (I64, I32) => builder.ins().ireduce(target, val),
        (F32, F64) => builder.ins().fpromote(target, val),
        (F64, F32) => builder.ins().fdemote(target, val),
        (I32, F32) | (I32, F64) => builder.ins().fcvt_from_sint(target, val),
        (I64, F32) | (I64, F64) => builder.ins().fcvt_from_sint(target, val),
        (F32, I32) | (F64, I32) => builder.ins().fcvt_to_sint_sat(target, val),
        (F32, I64) | (F64, I64) => builder.ins().fcvt_to_sint_sat(target, val),
        _ => val,
    }
}

// ─── Zero constants ───────────────────────────────────────────────────────────

fn zero_val(builder: &mut FunctionBuilder<'_>, jt: JitType) -> CVal {
    match jt {
        JitType::I32 => builder.ins().iconst(I32, 0),
        JitType::I64 => builder.ins().iconst(I64, 0),
        JitType::F32 => builder.ins().f32const(0.0),
        JitType::F64 => builder.ins().f64const(0.0),
    }
}

// ─── Boolean helpers ──────────────────────────────────────────────────────────

fn is_truthy(builder: &mut FunctionBuilder<'_>, val: CVal) -> CVal {
    let ty = builder.func.dfg.value_type(val);
    let zero = builder.ins().iconst(ty, 0);
    builder.ins().icmp(IntCC::NotEqual, val, zero)
}

fn is_zero(builder: &mut FunctionBuilder<'_>, val: CVal) -> CVal {
    let ty = builder.func.dfg.value_type(val);
    let zero = builder.ins().iconst(ty, 0);
    builder.ins().icmp(IntCC::Equal, val, zero)
}

// ─── Label target helper ──────────────────────────────────────────────────────

fn label_target_ip(op: &Operand) -> Option<usize> {
    match op {
        Operand::LabelTarget(t) => Some(*t as usize),
        _ => None,
    }
}

// ─── Binary operation helper ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum BinOp {
    Add, Sub, Mul, Div, Mod, And, Or, Xor, Shl, Shr,
}

fn emit_binop(
    builder: &mut FunctionBuilder<'_>,
    instr: &fasm_bytecode::Instruction,
    vars: &HashMap<u8, Variable>,
    ret_var: &Variable,
    ret_hint: JitType,
    op: BinOp,
) -> Option<()> {
    let a = read_op(builder, instr.operands.first()?, vars, ret_var, ret_hint)?;
    let b_raw = read_op(builder, instr.operands.get(1)?, vars, ret_var, ret_hint)?;
    let ty = builder.func.dfg.value_type(a);
    let b = coerce_to(builder, b_raw, ty);
    let result = match op {
        BinOp::Add => if ty == F32 || ty == F64 { builder.ins().fadd(a, b) } else { builder.ins().iadd(a, b) },
        BinOp::Sub => if ty == F32 || ty == F64 { builder.ins().fsub(a, b) } else { builder.ins().isub(a, b) },
        BinOp::Mul => if ty == F32 || ty == F64 { builder.ins().fmul(a, b) } else { builder.ins().imul(a, b) },
        BinOp::Div => if ty == F32 || ty == F64 { builder.ins().fdiv(a, b) } else { builder.ins().sdiv(a, b) },
        BinOp::Mod => if ty == F32 || ty == F64 { return None } else { builder.ins().srem(a, b) },
        BinOp::And => builder.ins().band(a, b),
        BinOp::Or  => builder.ins().bor(a, b),
        BinOp::Xor => builder.ins().bxor(a, b),
        BinOp::Shl => builder.ins().ishl(a, b),
        BinOp::Shr => builder.ins().sshr(a, b),
    };
    write_op(builder, instr.operands.get(2)?, result, vars, ret_var)
}

// ─── Comparison helper ────────────────────────────────────────────────────────

fn emit_cmp(
    builder: &mut FunctionBuilder<'_>,
    instr: &fasm_bytecode::Instruction,
    vars: &HashMap<u8, Variable>,
    ret_var: &Variable,
    ret_hint: JitType,
    cc: IntCC,
) -> Option<()> {
    let a = read_op(builder, instr.operands.first()?, vars, ret_var, ret_hint)?;
    let b_raw = read_op(builder, instr.operands.get(1)?, vars, ret_var, ret_hint)?;
    let ty = builder.func.dfg.value_type(a);
    let b = coerce_to(builder, b_raw, ty);
    // Produce i1 → extend to i32 (Bool convention in FASM).
    let cmp = builder.ins().icmp(cc, a, b);
    let as_i32 = builder.ins().uextend(I32, cmp);
    write_op(builder, instr.operands.get(2)?, as_i32, vars, ret_var)
}

// ─── Call-boundary value pack/unpack ─────────────────────────────────────────

/// Pack a `Value::Struct` into a flat `Vec<i64>` in PARAM declaration order.
pub fn pack_args(args: &Value, params: &[(u32, JitType)]) -> Option<Vec<i64>> {
    let s = match args {
        Value::Struct(s) => s,
        _ => return None,
    };
    let mut out = Vec::with_capacity(params.len());
    for &(key, _jt) in params {
        let v = s.get(&key)?;
        let bits = value_to_i64(v)?;
        out.push(bits);
    }
    Some(out)
}

/// Unpack a raw `i64` return value from JIT into a `Value`.
pub fn unpack_ret(raw: i64, ret_type: JitType) -> Value {
    match ret_type {
        JitType::I32 => Value::Int32(raw as i32),
        JitType::I64 => Value::Int64(raw),
        JitType::F32 => Value::Float32(f32::from_bits(raw as u32)),
        JitType::F64 => Value::Float64(f64::from_bits(raw as u64)),
    }
}

fn value_to_i64(v: &Value) -> Option<i64> {
    Some(match v {
        Value::Bool(b)   => *b as i64,
        Value::Int8(n)   => *n as i64,
        Value::Int16(n)  => *n as i64,
        Value::Int32(n)  => *n as i64,
        Value::Int64(n)  => *n,
        Value::Uint8(n)  => *n as i64,
        Value::Uint16(n) => *n as i64,
        Value::Uint32(n) => *n as i64,
        Value::Uint64(n) => *n as i64,
        Value::Float32(f) => f.to_bits() as i64,
        Value::Float64(f) => f.to_bits() as i64,
        _ => return None,
    })
}

/// Invoke a JIT-compiled function.
///
/// # Safety
/// `fn_ptr` must be a valid function pointer produced by `compile_program`.
pub unsafe fn call_jit(entry: &JitEntry, args: &Value) -> Option<Value> {
    let packed = pack_args(args, &entry.params)?;
    let n = packed.len();

    let raw: i64 = match n {
        0 => {
            let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(entry.fn_ptr);
            f()
        }
        1 => {
            let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0])
        }
        2 => {
            let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1])
        }
        3 => {
            let f: unsafe extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2])
        }
        4 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2], packed[3])
        }
        5 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2], packed[3], packed[4])
        }
        6 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2], packed[3], packed[4], packed[5])
        }
        7 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2], packed[3], packed[4], packed[5], packed[6])
        }
        8 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(entry.fn_ptr);
            f(packed[0], packed[1], packed[2], packed[3], packed[4], packed[5], packed[6], packed[7])
        }
        _ => return None, // >8 args: fall back to interpreter
    };

    Some(unpack_ret(raw, entry.ret_type))
}
