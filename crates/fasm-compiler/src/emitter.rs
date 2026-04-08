use std::collections::HashMap;
use fasm_bytecode::{
    Program, FunctionDef, ParamDescriptor,
    instruction::{Instruction, Operand, SlotRef, BuiltIn, Immediate},
    opcode::Opcode,
    types::FasmType,
};
use crate::ast::*;

pub fn compile(source: &str) -> Result<Program, String> {
    let tokens = crate::lexer::tokenize(source)?;
    let ast    = crate::parser::parse(tokens)?;
    crate::validator::validate(&ast)?;
    emit(ast)
}

pub fn emit(prog: ProgramAst) -> Result<Program, String> {
    let mut out = Program::new();

    // Build a DEFINE map for constant resolution
    let mut defines: HashMap<String, AstValue> = HashMap::new();
    for d in &prog.defines {
        defines.insert(d.name.clone(), d.value.clone());
    }

    // Global reserves
    for gr in &prog.global_reserves {
        let t = gr.fasm_type;
        let init_op = ast_value_to_operand(&gr.init, &defines, &HashMap::new(), &HashMap::new(), &[]);
        out.global_inits.push(Instruction {
            opcode: Opcode::Reserve,
            operands: vec![
                Operand::Key(gr.index),
                Operand::Type(t),
                init_op,
            ],
        });
    }

    // Functions
    let func_index_map: HashMap<String, u16> = prog.functions.iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i as u16))
        .collect();

    for func in prog.functions {
        let mut ctx = FuncCtx::new(&defines, &func_index_map);

        // Register params in symbol table
        for p in &func.params {
            let key = resolve_u32(&p.key, &defines);
            ctx.params.insert(p.name.clone(), key);
        }

        // First pass: collect locals and label positions to resolve forward refs
        // We do a two-pass emit: first collect locals+labels, then emit instrs
        let mut locals: HashMap<String, u8> = HashMap::new();
        collect_locals_and_labels(&func.body, &mut locals, &mut ctx.label_positions, &mut (0u32));

        ctx.locals = locals;

        // Emit param descriptors
        let mut params = Vec::new();
        for p in &func.params {
            let key = resolve_u32(&p.key, &defines);
            params.push(ParamDescriptor {
                key,
                fasm_type: p.fasm_type,
                name: p.name.clone(),
                required: p.required,
            });
        }

        // Emit instructions
        let mut instructions = Vec::new();
        emit_statements(&func.body, &mut instructions, &mut ctx)?;

        out.functions.push(FunctionDef { name: func.name, params, instructions });
    }

    Ok(out)
}

// ── Function emission context ──────────────────────────────────────────────

struct FuncCtx<'a> {
    defines: &'a HashMap<String, AstValue>,
    func_index_map: &'a HashMap<String, u16>,
    locals: HashMap<String, u8>,
    params: HashMap<String, u32>,   // param name -> key
    label_positions: HashMap<String, u32>,
}

impl<'a> FuncCtx<'a> {
    fn new(defines: &'a HashMap<String, AstValue>, func_index_map: &'a HashMap<String, u16>) -> Self {
        Self {
            defines,
            func_index_map,
            locals: HashMap::new(),
            params: HashMap::new(),
            label_positions: HashMap::new(),
        }
    }
}

// ── Two-pass helpers ───────────────────────────────────────────────────────

fn collect_locals_and_labels(stmts: &[Statement], locals: &mut HashMap<String, u8>, labels: &mut HashMap<String, u32>, ip: &mut u32) {
    for stmt in stmts {
        match stmt {
            Statement::Local(decl) => {
                locals.insert(decl.name.clone(), decl.index);
                // LOCAL emits a RESERVE instruction
                *ip += 1;
            }
            Statement::Label(name, _) => {
                labels.insert(name.clone(), *ip);
            }
            Statement::Instr(_) => { *ip += 1; }
            Statement::TryBlock { body, catch_body, catch_label, end_label, .. } => {
                // TRY emits one instruction
                *ip += 1;
                collect_locals_and_labels(body, locals, labels, ip);
                // CATCH label points here
                labels.insert(catch_label.clone(), *ip);
                // CATCH instruction (jump-over)
                *ip += 1;
                collect_locals_and_labels(catch_body, locals, labels, ip);
                // ENDTRY label
                labels.insert(end_label.clone(), *ip);
                *ip += 1;
            }
        }
    }
}

// ── Statement emission ─────────────────────────────────────────────────────

fn emit_statements(stmts: &[Statement], out: &mut Vec<Instruction>, ctx: &mut FuncCtx) -> Result<(), String> {
    for stmt in stmts {
        emit_statement(stmt, out, ctx)?;
    }
    Ok(())
}

fn emit_statement(stmt: &Statement, out: &mut Vec<Instruction>, ctx: &mut FuncCtx) -> Result<(), String> {
    match stmt {
        Statement::Local(decl) => {
            // LOCAL emits RESERVE local_idx, type, NULL
            out.push(Instruction {
                opcode: Opcode::Reserve,
                operands: vec![
                    Operand::Slot(SlotRef::Local(decl.index)),
                    Operand::Type(decl.fasm_type),
                    Operand::Imm(Immediate::Null),
                ],
            });
        }
        Statement::Label(_name, _line) => {
            // Labels are positional markers — no instruction emitted
        }
        Statement::Instr(instr) => {
            emit_instr(instr, out, ctx)?;
        }
        Statement::TryBlock { catch_label, end_label, body, catch_body, .. } => {
            let catch_ip = *ctx.label_positions.get(catch_label).unwrap_or(&0);
            let end_ip   = *ctx.label_positions.get(end_label).unwrap_or(&0);
            out.push(Instruction {
                opcode: Opcode::Try,
                operands: vec![
                    Operand::LabelTarget(catch_ip),
                    Operand::LabelTarget(end_ip),
                ],
            });
            emit_statements(body, out, ctx)?;
            // CATCH instruction — normal path skips over catch body to ENDTRY
            out.push(Instruction {
                opcode: Opcode::Catch,
                operands: vec![Operand::LabelTarget(end_ip)],
            });
            emit_statements(catch_body, out, ctx)?;
            out.push(Instruction::no_args(Opcode::EndTry));
        }
    }
    Ok(())
}

// ── Instruction emission ───────────────────────────────────────────────────

fn emit_instr(instr: &Instr, out: &mut Vec<Instruction>, ctx: &mut FuncCtx) -> Result<(), String> {
    let ln = instr.line;
    let ops = &instr.operands;

    macro_rules! op {
        ($n:expr) => {
            ops.get($n).ok_or_else(|| format!("Line {}: '{}' missing operand {}", ln, instr.mnemonic, $n))?
        };
    }

    macro_rules! slot {
        ($n:expr) => {
            ast_val_to_slot(op!($n), ctx).ok_or_else(|| format!("Line {}: cannot resolve slot operand {}", ln, $n))?
        };
    }

    macro_rules! val_op {
        ($n:expr) => {
            ast_value_to_operand(op!($n), ctx.defines, &ctx.locals, &ctx.params, &[])
        };
    }

    let opcode: Opcode = match instr.mnemonic.as_str() {
        // ── Memory ──────────────────────────────────────────────────────────
        "RESERVE" => {
            let idx_op = slot!(0);
            let t = ast_val_type(op!(1))?;
            let init = val_op!(2);
            out.push(Instruction { opcode: Opcode::Reserve, operands: vec![Operand::Slot(idx_op), Operand::Type(t), init] });
            return Ok(());
        }
        "RELEASE" => {
            let idx_op = slot!(0);
            out.push(Instruction { opcode: Opcode::Release, operands: vec![Operand::Slot(idx_op)] });
            return Ok(());
        }

        // ── Data movement ────────────────────────────────────────────────────
        "MOV" => {
            let src = val_op!(0);
            let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Mov, operands: vec![src, Operand::Slot(tgt)] });
            return Ok(());
        }
        "STORE" => {
            let src = val_op!(0);
            let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Store, operands: vec![src, Operand::Slot(tgt)] });
            return Ok(());
        }
        "ADDR" => {
            let src = slot!(0);
            let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Addr, operands: vec![Operand::Slot(src), Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Arithmetic ───────────────────────────────────────────────────────
        "ADD" => Opcode::Add,
        "SUB" => Opcode::Sub,
        "MUL" => Opcode::Mul,
        "DIV" => Opcode::Div,
        "MOD" => Opcode::Mod,

        "NEG" => {
            let src = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Neg, operands: vec![src, Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Comparison ───────────────────────────────────────────────────────
        "EQ"  => Opcode::Eq,
        "NEQ" => Opcode::Neq,
        "LT"  => Opcode::Lt,
        "LTE" => Opcode::Lte,
        "GT"  => Opcode::Gt,
        "GTE" => Opcode::Gte,

        // ── Bitwise ──────────────────────────────────────────────────────────
        "AND" => Opcode::And,
        "OR"  => Opcode::Or,
        "XOR" => Opcode::Xor,
        "SHL" => Opcode::Shl,
        "SHR" => Opcode::Shr,
        "NOT" => {
            let src = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Not, operands: vec![src, Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Control flow ─────────────────────────────────────────────────────
        "JMP" => {
            let tgt = label_op(op!(0), &ctx.label_positions, ln)?;
            out.push(Instruction { opcode: Opcode::Jmp, operands: vec![tgt] });
            return Ok(());
        }
        "JZ" => {
            let cond = val_op!(0);
            let tgt  = label_op(op!(1), &ctx.label_positions, ln)?;
            out.push(Instruction { opcode: Opcode::Jz, operands: vec![cond, tgt] });
            return Ok(());
        }
        "JNZ" => {
            let cond = val_op!(0);
            let tgt  = label_op(op!(1), &ctx.label_positions, ln)?;
            out.push(Instruction { opcode: Opcode::Jnz, operands: vec![cond, tgt] });
            return Ok(());
        }
        "CALL" | "ASYNC_CALL" | "TAIL_CALL" => {
            let func_name = ast_ident(op!(0))?;
            let func_idx = ctx.func_index_map.get(&func_name)
                .copied()
                .ok_or_else(|| format!("Line {}: undefined function '{}'", ln, func_name))?;
            let args_op = val_op!(1);
            let opcode = match instr.mnemonic.as_str() {
                "CALL" => Opcode::Call,
                "TAIL_CALL" => Opcode::TailCall,
                _ => Opcode::AsyncCall,
            };
            out.push(Instruction { opcode, operands: vec![Operand::FuncRef(func_idx), args_op] });
            return Ok(());
        }
        "RET" => {
            if ops.is_empty() {
                out.push(Instruction::no_args(Opcode::Ret));
            } else {
                let val = val_op!(0);
                out.push(Instruction { opcode: Opcode::Ret, operands: vec![val] });
            }
            return Ok(());
        }
        "HALT" => {
            out.push(Instruction::no_args(Opcode::Halt));
            return Ok(());
        }
        "AWAIT" => {
            let future = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Await, operands: vec![future, Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Syscall ──────────────────────────────────────────────────────────
        "SYSCALL" | "ASYNC_SYSCALL" => {
            let id = resolve_syscall_id(op!(0), ctx.defines, ln)?;
            let args = val_op!(1);
            let opcode = if instr.mnemonic == "SYSCALL" { Opcode::Syscall } else { Opcode::AsyncSyscall };
            out.push(Instruction { opcode, operands: vec![Operand::SyscallId(id), args] });
            return Ok(());
        }

        // ── Collections ──────────────────────────────────────────────────────
        "PUSH" => {
            let coll = slot!(0); let val = val_op!(1);
            out.push(Instruction { opcode: Opcode::Push, operands: vec![Operand::Slot(coll), val] });
            return Ok(());
        }
        "POP" => {
            let coll = slot!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Pop, operands: vec![Operand::Slot(coll), Operand::Slot(tgt)] });
            return Ok(());
        }
        "ENQUEUE" => {
            let q = slot!(0); let val = val_op!(1);
            out.push(Instruction { opcode: Opcode::Enqueue, operands: vec![Operand::Slot(q), val] });
            return Ok(());
        }
        "DEQUEUE" => {
            let q = slot!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Dequeue, operands: vec![Operand::Slot(q), Operand::Slot(tgt)] });
            return Ok(());
        }
        "PEEK" => {
            let coll = slot!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Peek, operands: vec![Operand::Slot(coll), Operand::Slot(tgt)] });
            return Ok(());
        }
        "GET_IDX" => {
            let coll = val_op!(0); let idx = val_op!(1); let tgt = slot!(2);
            out.push(Instruction { opcode: Opcode::GetIdx, operands: vec![coll, idx, Operand::Slot(tgt)] });
            return Ok(());
        }
        "SET_IDX" => {
            let coll = slot!(0); let idx = val_op!(1); let val = val_op!(2);
            out.push(Instruction { opcode: Opcode::SetIdx, operands: vec![Operand::Slot(coll), idx, val] });
            return Ok(());
        }
        "GET_FIELD" => {
            let coll = val_op!(0); let key = key_op(op!(1), ctx.defines, ln)?; let tgt = slot!(2);
            out.push(Instruction { opcode: Opcode::GetField, operands: vec![coll, key, Operand::Slot(tgt)] });
            return Ok(());
        }
        "SET_FIELD" => {
            let coll = slot!(0); let key = key_op(op!(1), ctx.defines, ln)?; let val = val_op!(2);
            out.push(Instruction { opcode: Opcode::SetField, operands: vec![Operand::Slot(coll), key, val] });
            return Ok(());
        }
        "HAS_FIELD" => {
            let coll = val_op!(0); let key = key_op(op!(1), ctx.defines, ln)?; let tgt = slot!(2);
            out.push(Instruction { opcode: Opcode::HasField, operands: vec![coll, key, Operand::Slot(tgt)] });
            return Ok(());
        }
        "DEL_FIELD" => {
            let coll = slot!(0); let key = key_op(op!(1), ctx.defines, ln)?;
            out.push(Instruction { opcode: Opcode::DelField, operands: vec![Operand::Slot(coll), key] });
            return Ok(());
        }
        "LEN" => {
            let coll = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Len, operands: vec![coll, Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Wrapper instructions ──────────────────────────────────────────────
        "SOME" => {
            let tgt = slot!(0); let val = val_op!(1);
            out.push(Instruction { opcode: Opcode::Some_, operands: vec![Operand::Slot(tgt), val] });
            return Ok(());
        }
        "IS_SOME" => {
            let opt = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::IsSome, operands: vec![opt, Operand::Slot(tgt)] });
            return Ok(());
        }
        "UNWRAP" => {
            let opt = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::Unwrap, operands: vec![opt, Operand::Slot(tgt)] });
            return Ok(());
        }
        "OK" => {
            let tgt = slot!(0); let val = val_op!(1);
            out.push(Instruction { opcode: Opcode::Ok_, operands: vec![Operand::Slot(tgt), val] });
            return Ok(());
        }
        "ERR" => {
            let tgt = slot!(0); let code = val_op!(1);
            out.push(Instruction { opcode: Opcode::Err_, operands: vec![Operand::Slot(tgt), code] });
            return Ok(());
        }
        "IS_OK" => {
            let res = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::IsOk, operands: vec![res, Operand::Slot(tgt)] });
            return Ok(());
        }
        "UNWRAP_OK" => {
            let res = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::UnwrapOk, operands: vec![res, Operand::Slot(tgt)] });
            return Ok(());
        }
        "UNWRAP_ERR" => {
            let res = val_op!(0); let tgt = slot!(1);
            out.push(Instruction { opcode: Opcode::UnwrapErr, operands: vec![res, Operand::Slot(tgt)] });
            return Ok(());
        }

        // ── Cast ─────────────────────────────────────────────────────────────
        "CAST" => {
            let src = val_op!(0);
            let t   = ast_val_type(op!(1))?;
            let tgt = slot!(2);
            out.push(Instruction { opcode: Opcode::Cast, operands: vec![src, Operand::Type(t), Operand::Slot(tgt)] });
            return Ok(());
        }

        _ => return Err(format!("Line {}: unknown instruction '{}'", ln, instr.mnemonic)),
    };

    // Generic 3-operand: src, src, target
    let a = val_op!(0);
    let b = val_op!(1);
    let t = slot!(2);
    out.push(Instruction { opcode, operands: vec![a, b, Operand::Slot(t)] });
    Ok(())
}

// ── Resolution helpers ─────────────────────────────────────────────────────

fn ast_val_to_slot(val: &AstValue, ctx: &FuncCtx) -> Option<SlotRef> {
    match val {
        AstValue::Deref(name) => {
            // &name -> deref local
            if let Some(&idx) = ctx.locals.get(name) {
                return Some(SlotRef::DerefLocal(idx));
            }
            None
        }
        AstValue::Ident(name) => {
            match name.as_str() {
                "$args"        => Some(SlotRef::BuiltIn(BuiltIn::Args)),
                "$ret"         => Some(SlotRef::BuiltIn(BuiltIn::Ret)),
                "$fault_code"  => Some(SlotRef::BuiltIn(BuiltIn::FaultCode)),
                "$fault_index" => Some(SlotRef::BuiltIn(BuiltIn::FaultIndex)),
                _ => {
                    // Local?
                    if let Some(&idx) = ctx.locals.get(name) {
                        return Some(SlotRef::Local(idx));
                    }
                    // DEFINE that is a number?
                    if let Some(def_val) = ctx.defines.get(name) {
                        if let AstValue::Integer(n) = def_val {
                            return Some(SlotRef::Local(*n as u8));
                        }
                    }
                    None
                }
            }
        }
        AstValue::Integer(n) => Some(SlotRef::Local(*n as u8)),
        AstValue::HexInt(n)  => Some(SlotRef::Local(*n as u8)),
        _ => None,
    }
}

fn ast_value_to_operand(
    val: &AstValue,
    defines: &HashMap<String, AstValue>,
    locals: &HashMap<String, u8>,
    params: &HashMap<String, u32>,
    _extra: &[()],
) -> Operand {
    match val {
        AstValue::Null  => Operand::Imm(Immediate::Null),
        AstValue::True  => Operand::Imm(Immediate::Bool(true)),
        AstValue::False => Operand::Imm(Immediate::Bool(false)),
        AstValue::Integer(n) => Operand::Imm(Immediate::Int32(*n as i32)),
        AstValue::HexInt(n)  => Operand::Imm(Immediate::Uint32(*n as u32)),
        AstValue::Float(f)   => Operand::Imm(Immediate::Float64(*f)),
        // String literals compile to VEC<UINT8> at runtime via Immediate::Str
        AstValue::Str(s) => Operand::Imm(Immediate::Str(s.clone())),
        AstValue::Deref(name) => {
            if let Some(&idx) = locals.get(name) {
                Operand::Slot(SlotRef::DerefLocal(idx))
            } else {
                Operand::Slot(SlotRef::DerefLocal(0))
            }
        }
        AstValue::Ident(name) => match name.as_str() {
            "$args"        => Operand::Slot(SlotRef::BuiltIn(BuiltIn::Args)),
            "$ret"         => Operand::Slot(SlotRef::BuiltIn(BuiltIn::Ret)),
            "$fault_code"  => Operand::Slot(SlotRef::BuiltIn(BuiltIn::FaultCode)),
            "$fault_index" => Operand::Slot(SlotRef::BuiltIn(BuiltIn::FaultIndex)),
            _ => {
                // Local name?
                if let Some(&idx) = locals.get(name) {
                    return Operand::Slot(SlotRef::Local(idx));
                }
                // DEFINE constant?
                if let Some(def_val) = defines.get(name) {
                    return ast_value_to_operand(def_val, defines, locals, params, _extra);
                }
                // Integer literal that was parsed as ident? Shouldn't happen but fallback
                if let Ok(n) = name.parse::<i64>() {
                    return Operand::Imm(Immediate::Int32(n as i32));
                }
                // Unknown — emit as local 0 (will fault at runtime if wrong)
                Operand::Slot(SlotRef::Local(0))
            }
        }
    }
}

fn ast_val_type(val: &AstValue) -> Result<FasmType, String> {
    match val {
        AstValue::Ident(s) => {
            use fasm_bytecode::types::FasmType::*;
            match s.as_str() {
                "BOOL"     => Ok(Bool),
                "INT8"     => Ok(Int8),
                "INT16"    => Ok(Int16),
                "INT32"    => Ok(Int32),
                "INT64"    => Ok(Int64),
                "UINT8"    => Ok(Uint8),
                "UINT16"   => Ok(Uint16),
                "UINT32"   => Ok(Uint32),
                "UINT64"   => Ok(Uint64),
                "FLOAT32"  => Ok(Float32),
                "FLOAT64"  => Ok(Float64),
                "REF_MUT"  => Ok(RefMut),
                "REF_IMM"  => Ok(RefImm),
                "VEC"      => Ok(Vec),
                "STRUCT"   => Ok(Struct),
                "STACK"    => Ok(Stack),
                "QUEUE"    => Ok(Queue),
                "HEAP_MIN" => Ok(HeapMin),
                "HEAP_MAX" => Ok(HeapMax),
                "OPTION"   => Ok(Option),
                "RESULT"   => Ok(Result),
                "FUTURE"   => Ok(Future),
                "NULL"     => Ok(Null),
                _ => Err(format!("Unknown type '{}'", s)),
            }
        }
        _ => Err("Expected type name".into()),
    }
}

fn label_op(val: &AstValue, labels: &HashMap<String, u32>, ln: usize) -> Result<Operand, String> {
    match val {
        AstValue::Ident(name) => {
            let pos = labels.get(name)
                .copied()
                .ok_or_else(|| format!("Line {}: undefined label '{}'", ln, name))?;
            Ok(Operand::LabelTarget(pos))
        }
        _ => Err(format!("Line {}: expected label name", ln)),
    }
}

fn key_op(val: &AstValue, defines: &HashMap<String, AstValue>, _ln: usize) -> Result<Operand, String> {
    let k = resolve_u32(val, defines);
    Ok(Operand::Key(k))
}

fn resolve_u32(val: &AstValue, defines: &HashMap<String, AstValue>) -> u32 {
    match val {
        AstValue::Integer(n)  => *n as u32,
        AstValue::HexInt(n)   => *n as u32,
        AstValue::Ident(name) => {
            if let Some(def_val) = defines.get(name) {
                resolve_u32(def_val, defines)
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn resolve_syscall_id(val: &AstValue, defines: &HashMap<String, AstValue>, ln: usize) -> Result<i32, String> {
    match val {
        AstValue::Integer(n) => Ok(*n as i32),
        AstValue::HexInt(n)  => Ok(*n as i32),
        AstValue::Ident(name) => {
            if let Some(def_val) = defines.get(name) {
                resolve_syscall_id(def_val, defines, ln)
            } else {
                Err(format!("Line {}: undefined DEFINE for syscall id '{}'", ln, name))
            }
        }
        _ => Err(format!("Line {}: expected syscall id", ln)),
    }
}

fn ast_ident(val: &AstValue) -> Result<String, String> {
    match val {
        AstValue::Ident(s) => Ok(s.clone()),
        _ => Err("Expected identifier".into()),
    }
}
