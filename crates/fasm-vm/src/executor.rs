use crate::{
    fault::Fault,
    memory::{Frame, GlobalRegister},
    value::{
        numeric_as_i64, numeric_as_u64, FasmBTree, FasmBitset, FasmBitvec, FasmDeque, FasmHeapMax,
        FasmHeapMin, FasmOption, FasmQueue, FasmResult, FasmSlice, FasmSparse, FasmStack,
        FasmStruct, FasmVec, Value,
    },
};
use fasm_bytecode::{
    instruction::{BuiltIn, Immediate, Operand, SlotRef},
    opcode::Opcode,
    types::FasmType,
    Program,
};
use std::collections::HashMap;

const MAX_CALL_DEPTH: usize = 512;

/// Snapshot used by TRY/CATCH for transactional rollback.
struct TryGuard {
    catch_ip: usize, // instruction index of CATCH
    end_ip: usize,   // instruction index of ENDTRY
    frame_snap: Vec<Option<Value>>,
    global_snap: Vec<Option<Value>>,
}

/// One entry on the call stack.
struct CallFrame {
    func_name: String,
    ip: usize,
    frame: Frame,
    args: Value,    // the incoming STRUCT ($args)
    ret_val: Value, // $ret
    try_guard: Option<TryGuard>,
    tmp_frames: Vec<crate::memory::TmpFrame>,
}

impl CallFrame {
    fn new(func_name: String, args: Value) -> Self {
        Self {
            func_name,
            ip: 0,
            frame: Frame::new(),
            args,
            ret_val: Value::Null,
            try_guard: None,
            tmp_frames: vec![],
        }
    }
}

/// A syscall handler signature.
pub type SyscallFn = Box<dyn Fn(Value, &mut GlobalRegister) -> Result<Value, Fault> + Send + Sync>;

/// The main VM executor.
pub struct Executor {
    pub globals: GlobalRegister,
    syscalls: HashMap<i32, SyscallFn>,
    call_stack: Vec<CallFrame>,
}

impl Executor {
    pub fn new() -> Self {
        let mut ex = Self {
            globals: GlobalRegister::new(),
            syscalls: HashMap::new(),
            call_stack: Vec::new(),
        };
        ex.register_builtins();
        ex
    }

    fn register_builtins(&mut self) {
        // 0 = PRINT: struct key 0 = value to print, or just print the value directly
        self.syscalls.insert(
            0,
            Box::new(|args, _globals| {
                let v = if let Value::Struct(s) = &args {
                    s.get(&0).cloned().unwrap_or_else(|| args.clone())
                } else {
                    args.clone()
                };
                println!("{}", v.display());
                Ok(Value::Null)
            }),
        );
        // 1 = PRINT_VEC: struct key 0 = VEC to print as text, or just print the value directly
        self.syscalls.insert(
            1,
            Box::new(|args, _globals| {
                let v = if let Value::Struct(s) = &args {
                    s.get(&0).cloned().unwrap_or_else(|| args.clone())
                } else {
                    args.clone()
                };
                print!("{}", v.display());
                Ok(Value::Null)
            }),
        );
        // 2 = READ: reads a line from stdin, returns VEC<UINT8>.
        // Trailing whitespace (CR, LF, trailing spaces) is stripped so that
        // callers never need to sanitise the result themselves.
        self.syscalls.insert(
            2,
            Box::new(|_args, _globals| {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).ok();
                let bytes: Vec<Value> = line.trim_end().bytes().map(Value::Uint8).collect();
                Ok(Value::Vec(FasmVec(bytes)))
            }),
        );
        // 3 = EXIT: struct key 0 = exit code
        self.syscalls.insert(
            3,
            Box::new(|args, _globals| {
                let code = if let Value::Struct(s) = &args {
                    match s.get(&0) {
                        Some(Value::Int32(n)) => *n,
                        _ => 0,
                    }
                } else {
                    0
                };
                std::process::exit(code);
            }),
        );
        // 4 = PARSE_INT: struct key 0 = VEC<UINT8> of ASCII digits (optional leading '-')
        //     Returns RESULT<INT32>: Ok(n) on success, Err(1) on invalid input.
        //     Mirrors C's atoi / strtol. Used by calculator.fasm so it doesn't need
        //     to implement digit parsing manually.
        self.syscalls.insert(
            4,
            Box::new(|args, _globals| {
                const ERR_BAD_INPUT: u32 = 1;
                let bytes = if let Value::Struct(s) = &args {
                    match s.get(&0) {
                        Some(Value::Vec(v)) => v.0.clone(),
                        _ => return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT)))),
                    }
                } else {
                    return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT))));
                };

                // Collect raw bytes
                let raw: Option<Vec<u8>> = bytes
                    .iter()
                    .map(|v| match v {
                        Value::Uint8(b) => Some(*b),
                        _ => None,
                    })
                    .collect();
                let raw = match raw {
                    Some(r) => r,
                    None => return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT)))),
                };

                // Parse to string and trim leading/trailing whitespace (CR, LF, spaces).
                // Whitespace *within* the string is intentionally left in so it
                // causes a parse failure (e.g. "1 2" is invalid).
                let s = String::from_utf8_lossy(&raw);
                let trimmed = s.trim().as_bytes().to_vec();
                let raw = trimmed;

                if raw.is_empty() {
                    return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT))));
                }

                // Parse sign
                let (sign, digits) = if raw[0] == b'-' {
                    (-1i64, &raw[1..])
                } else {
                    (1i64, &raw[..])
                };

                if digits.is_empty() {
                    return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT))));
                }

                let mut result: i64 = 0;
                for &b in digits {
                    if !b.is_ascii_digit() {
                        return Ok(Value::Result(Box::new(FasmResult::Err(ERR_BAD_INPUT))));
                    }
                    result = result * 10 + (b - b'0') as i64;
                }

                Ok(Value::Result(Box::new(FasmResult::Ok(Value::Int32(
                    (result * sign) as i32,
                )))))
            }),
        );
    }

    pub fn mount_syscall(&mut self, id: i32, handler: SyscallFn) {
        self.syscalls.insert(id, handler);
    }

    /// Run a full program starting from `Main` with an empty args struct.
    pub fn run(&mut self, program: &Program) -> Result<Value, String> {
        let _ = program
            .get_function("Main")
            .ok_or("No 'Main' function found in program")?;
        self.run_named(program, "Main", Value::Struct(FasmStruct::default()))
    }

    /// Run a full program starting from a named entry-point function.
    ///
    /// `args` is passed as the function's `$args` struct, enabling HTTP handlers,
    /// scheduled tasks, and event handlers to be regular FASM functions with typed
    /// `PARAM` declarations. Global variables are initialised before the call.
    pub fn run_named(
        &mut self,
        program: &Program,
        func: &str,
        args: Value,
    ) -> Result<Value, String> {
        // Execute global inits (idempotent — engine may call multiple handlers
        // on the same executor instance, re-running inits is harmless because
        // Reserve is a set, not an append).
        for instr in &program.global_inits {
            if let Opcode::Reserve = instr.opcode {
                if let (Some(Operand::Key(idx)), Some(Operand::Type(t)), Some(init)) = (
                    instr.operands.first(),
                    instr.operands.get(1),
                    instr.operands.get(2),
                ) {
                    let val = imm_to_value_for_type(*t, init);
                    self.globals.set(*idx, val);
                }
            }
        }

        let _ = program
            .get_function(func)
            .ok_or_else(|| format!("No '{}' function found in program", func))?;

        self.call_stack.push(CallFrame::new(func.to_string(), args));

        loop {
            let frame_idx = self.call_stack.len() - 1;
            let ip = self.call_stack[frame_idx].ip;
            let func_name = self.call_stack[frame_idx].func_name.clone();

            let func_def = program
                .get_function(&func_name)
                .ok_or_else(|| format!("Function '{}' not found", func_name))?;

            if ip >= func_def.instructions.len() {
                // Implicit HALT / void return at ENDF
                let ret = self
                    .call_stack
                    .last()
                    .map(|f| f.ret_val.clone())
                    .unwrap_or(Value::Null);
                self.call_stack.pop();
                if self.call_stack.is_empty() {
                    return Ok(ret);
                }
                // propagate $ret to caller
                self.call_stack.last_mut().unwrap().ret_val = ret;
                continue;
            }

            let instr = func_def.instructions[ip].clone();
            self.call_stack[frame_idx].ip += 1;

            match self.execute_instruction(&instr, program) {
                Ok(action) => {
                    match action {
                        Action::Continue => {}
                        Action::Jump(target) => {
                            self.call_stack.last_mut().unwrap().ip = target;
                        }
                        Action::CallFunc(name, args) => {
                            if self.call_stack.len() >= MAX_CALL_DEPTH {
                                return Err(format!("{}", Fault::StackOverflow));
                            }
                            self.call_stack.push(CallFrame::new(name, args));
                        }
                        Action::TailCall(name, args) => {
                            let frame = self.call_stack.last_mut().unwrap();
                            frame.func_name = name;
                            frame.ip = 0;
                            frame.frame = Frame::new();
                            frame.args = args;
                            frame.ret_val = Value::Null;
                            // Keep try_guard active so try bounds transcend TCO safely.
                        }
                        Action::Return(val) => {
                            self.call_stack.pop();
                            if self.call_stack.is_empty() {
                                return Ok(val);
                            }
                            self.call_stack.last_mut().unwrap().ret_val = val;
                        }
                        Action::Halt => return Ok(Value::Null),
                    }
                }
                Err(fault) => {
                    // Check for TRY guard
                    let guard = self.call_stack.last_mut().unwrap().try_guard.take();
                    if let Some(g) = guard {
                        // Rollback memory
                        let frame = &mut self.call_stack.last_mut().unwrap().frame;
                        frame.restore(g.frame_snap);
                        self.globals.restore(g.global_snap);
                        // Set $fault_code via $ret
                        let cf = self.call_stack.last_mut().unwrap();
                        cf.ret_val = Value::Uint32(fault.code());
                        cf.ip = g.catch_ip;
                    } else {
                        return Err(format!(
                            "Unhandled fault in '{}' at ip {}: {}",
                            func_name, ip, fault
                        ));
                    }
                }
            }
        }
    }

    fn execute_instruction(
        &mut self,
        instr: &fasm_bytecode::Instruction,
        program: &Program,
    ) -> Result<Action, Fault> {
        let _cf_idx = self.call_stack.len() - 1;

        macro_rules! get_op {
            ($n:expr) => {
                instr.operands.get($n).ok_or(Fault::TypeMismatch)?
            };
        }

        macro_rules! read_val {
            ($op:expr) => {
                self.read_operand($op)?
            };
        }

        macro_rules! write_val {
            ($op:expr, $val:expr) => {
                self.write_operand($op, $val)?
            };
        }

        match &instr.opcode {
            Opcode::Nop => Ok(Action::Continue),

            Opcode::Halt => Ok(Action::Halt),

            // ── Memory ──────────────────────────────────────────────────────
            Opcode::TmpBlock => {
                let cf = self.call_stack.last_mut().unwrap();
                cf.tmp_frames.push(crate::memory::TmpFrame::new());
                Ok(Action::Continue)
            }
            Opcode::EndTmp => {
                let cf = self.call_stack.last_mut().unwrap();
                if cf.tmp_frames.pop().is_none() {
                    return Err(Fault::IndexOutOfBounds); // unmatched END_TMP
                }
                Ok(Action::Continue)
            }

            Opcode::Reserve => {
                let idx_op = get_op!(0);
                let type_op = get_op!(1);
                let init_op = get_op!(2);
                let t = if let Operand::Type(t) = type_op {
                    *t
                } else {
                    return Err(Fault::TypeMismatch);
                };
                let val = imm_to_value_for_type(t, init_op);
                write_val!(idx_op, val);
                Ok(Action::Continue)
            }

            Opcode::Release => {
                let idx_op = get_op!(0);
                self.release_operand(idx_op);
                Ok(Action::Continue)
            }

            // ── Data movement ────────────────────────────────────────────────
            Opcode::Mov => {
                let src = read_val!(get_op!(0));
                write_val!(get_op!(1), src);
                Ok(Action::Continue)
            }

            Opcode::Store => {
                let val = self.imm_operand(get_op!(0))?;
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }

            Opcode::Addr => {
                let (is_global, idx) = self.slot_address(get_op!(0));
                let target_op = get_op!(1);
                // Determine mutability based on target slot type if already reserved;
                // default to RefMut
                let ref_val = Value::RefMut(is_global, idx);
                write_val!(target_op, ref_val);
                Ok(Action::Continue)
            }

            // ── Arithmetic ───────────────────────────────────────────────────
            Opcode::Add => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.add(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), r);
                Ok(Action::Continue)
            }
            Opcode::Sub => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.sub(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), r);
                Ok(Action::Continue)
            }
            Opcode::Mul => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.mul(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), r);
                Ok(Action::Continue)
            }
            Opcode::Div => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                if !b.is_truthy() {
                    return Err(Fault::DivisionByZero);
                }
                let r = a.div(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), r);
                Ok(Action::Continue)
            }
            Opcode::Mod => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                if !b.is_truthy() {
                    return Err(Fault::DivisionByZero);
                }
                let r = a.rem(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), r);
                Ok(Action::Continue)
            }
            Opcode::Call | Opcode::AsyncCall | Opcode::TailCall => {
                let func_idx = match get_op!(0) {
                    Operand::FuncRef(idx) => *idx,
                    _ => return Err(Fault::TypeMismatch),
                };
                let name = program
                    .functions
                    .get(func_idx as usize)
                    .ok_or(Fault::UndeclaredSlot)?
                    .name
                    .clone();
                let args = read_val!(get_op!(1));

                if instr.opcode == Opcode::TailCall {
                    Ok(Action::TailCall(name.clone(), args))
                } else {
                    Ok(Action::CallFunc(name.clone(), args))
                }
            }
            Opcode::Neg => {
                let a = read_val!(get_op!(0));
                let r = a.neg().ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(1), r);
                Ok(Action::Continue)
            }

            // ── Comparison ───────────────────────────────────────────────────
            Opcode::Eq => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                write_val!(get_op!(2), Value::Bool(a.eq_val(&b)));
                Ok(Action::Continue)
            }
            Opcode::Neq => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                write_val!(get_op!(2), Value::Bool(!a.eq_val(&b)));
                Ok(Action::Continue)
            }
            Opcode::Lt => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.cmp_lt(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), Value::Bool(r));
                Ok(Action::Continue)
            }
            Opcode::Lte => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.cmp_lte(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), Value::Bool(r));
                Ok(Action::Continue)
            }
            Opcode::Gt => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.cmp_gt(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), Value::Bool(r));
                Ok(Action::Continue)
            }
            Opcode::Gte => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let r = a.cmp_gte(&b).ok_or(Fault::TypeMismatch)?;
                write_val!(get_op!(2), Value::Bool(r));
                Ok(Action::Continue)
            }

            // ── Bitwise ──────────────────────────────────────────────────────
            Opcode::And => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                write_val!(get_op!(2), a.bit_and(&b).ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }
            Opcode::Or => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                write_val!(get_op!(2), a.bit_or(&b).ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }
            Opcode::Xor => {
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                write_val!(get_op!(2), a.bit_xor(&b).ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }
            Opcode::Not => {
                let a = read_val!(get_op!(0));
                write_val!(get_op!(1), a.bit_not().ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }
            Opcode::Shl => {
                let a = read_val!(get_op!(0));
                let shift = read_val!(get_op!(1));
                let s = as_u32(&shift)?;
                write_val!(get_op!(2), a.shl(s).ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }
            Opcode::Shr => {
                let a = read_val!(get_op!(0));
                let shift = read_val!(get_op!(1));
                let s = as_u32(&shift)?;
                write_val!(get_op!(2), a.shr(s).ok_or(Fault::TypeMismatch)?);
                Ok(Action::Continue)
            }

            // ── Control flow ─────────────────────────────────────────────────
            Opcode::Jmp => {
                let target = label_target(get_op!(0))?;
                Ok(Action::Jump(target))
            }
            Opcode::Jz => {
                let cond = read_val!(get_op!(0));
                let target = label_target(get_op!(1))?;
                if !cond.is_truthy() {
                    Ok(Action::Jump(target))
                } else {
                    Ok(Action::Continue)
                }
            }
            Opcode::Jnz => {
                let cond = read_val!(get_op!(0));
                let target = label_target(get_op!(1))?;
                if cond.is_truthy() {
                    Ok(Action::Jump(target))
                } else {
                    Ok(Action::Continue)
                }
            }

            Opcode::Ret => {
                let val = if instr.operands.is_empty() {
                    Value::Null
                } else {
                    read_val!(get_op!(0))
                };
                Ok(Action::Return(val))
            }

            Opcode::Await => {
                // For MVP: the future is already resolved synchronously
                let future = read_val!(get_op!(0));
                let resolved = match future {
                    Value::Future(Some(v)) => *v,
                    _ => Value::Null,
                };
                write_val!(get_op!(1), resolved);
                Ok(Action::Continue)
            }

            // ── Syscall ──────────────────────────────────────────────────────
            Opcode::Syscall => {
                let id = syscall_id(get_op!(0))?;
                let args = read_val!(get_op!(1));
                // SAFETY: we take a raw pointer to the boxed fn before calling it so
                // that we can simultaneously hold &mut self.globals. The syscalls map
                // is not mutated during the call.
                let handler: *const SyscallFn = self.syscalls.get(&id).ok_or(Fault::BadSyscall)?;
                let result = unsafe { (*handler)(args, &mut self.globals) }?;
                self.call_stack.last_mut().unwrap().ret_val = result;
                Ok(Action::Continue)
            }
            Opcode::AsyncSyscall => {
                let id = syscall_id(get_op!(0))?;
                let args = read_val!(get_op!(1));
                let handler: *const SyscallFn = self.syscalls.get(&id).ok_or(Fault::BadSyscall)?;
                let result = unsafe { (*handler)(args, &mut self.globals) }?;
                self.call_stack.last_mut().unwrap().ret_val = Value::Future(Some(Box::new(result)));
                Ok(Action::Continue)
            }

            // ── Collections ──────────────────────────────────────────────────
            Opcode::Push => {
                let val = read_val!(get_op!(1));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Vec(v) => v.0.push(val),
                    Value::Stack(s) => s.0.push(val),
                    Value::Deque(d) => d.0.push_back(val),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::Pop => {
                let val = {
                    let coll = self.write_target_mut(get_op!(0))?;
                    match coll {
                        Value::Stack(s) => s.0.pop().ok_or(Fault::IndexOutOfBounds)?,
                        _ => return Err(Fault::TypeMismatch),
                    }
                };
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }
            Opcode::Enqueue => {
                let val = read_val!(get_op!(1));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Queue(q) => q.0.push_back(val),
                    Value::Deque(d) => d.0.push_back(val),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::Dequeue => {
                let val = {
                    let coll = self.write_target_mut(get_op!(0))?;
                    match coll {
                        Value::Queue(q) => q.0.pop_front().ok_or(Fault::IndexOutOfBounds)?,
                        Value::Deque(d) => d.0.pop_front().ok_or(Fault::IndexOutOfBounds)?,
                        _ => return Err(Fault::TypeMismatch),
                    }
                };
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }
            Opcode::Peek => {
                let coll = read_val!(get_op!(0));
                let val = match &coll {
                    Value::Stack(s) => s.0.last().cloned().ok_or(Fault::IndexOutOfBounds)?,
                    Value::Queue(q) => q.0.front().cloned().ok_or(Fault::IndexOutOfBounds)?,
                    Value::Deque(d) => d.0.front().cloned().ok_or(Fault::IndexOutOfBounds)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }
            Opcode::GetIdx => {
                let coll = read_val!(get_op!(0));
                let idx = as_u32(&read_val!(get_op!(1)))? as usize;
                let val = match &coll {
                    Value::Vec(v) => v.0.get(idx).cloned().ok_or(Fault::IndexOutOfBounds)?,
                    Value::HeapMin(h) => h.0.get(idx).cloned().ok_or(Fault::IndexOutOfBounds)?,
                    Value::HeapMax(h) => h.0.get(idx).cloned().ok_or(Fault::IndexOutOfBounds)?,
                    Value::Slice(s) => s.get(idx).cloned().ok_or(Fault::IndexOutOfBounds)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), val);
                Ok(Action::Continue)
            }
            Opcode::SetIdx => {
                let idx = as_u32(&read_val!(get_op!(1)))? as usize;
                let val = read_val!(get_op!(2));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Vec(v) => {
                        let slot = v.0.get_mut(idx).ok_or(Fault::IndexOutOfBounds)?;
                        *slot = val;
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::GetField => {
                let key = self.read_key_operand(get_op!(1))?;
                let coll = read_val!(get_op!(0));
                let val = match &coll {
                    Value::Struct(s) => s.get(&key).cloned().ok_or(Fault::FieldNotFound)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), val);
                Ok(Action::Continue)
            }
            Opcode::SetField => {
                let key = self.read_key_operand(get_op!(1))?;
                let val = read_val!(get_op!(2));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Struct(s) => {
                        s.insert(key, val);
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::HasField => {
                let key = self.read_key_operand(get_op!(1))?;
                let coll = read_val!(get_op!(0));
                let has = match &coll {
                    Value::Struct(s) => s.contains_key(&key),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), Value::Bool(has));
                Ok(Action::Continue)
            }
            Opcode::DelField => {
                let key = self.read_key_operand(get_op!(1))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Struct(s) => {
                        s.remove(&key);
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::Len => {
                let coll = read_val!(get_op!(0));
                let len = match &coll {
                    Value::Vec(v) => v.0.len() as u64,
                    Value::Stack(s) => s.0.len() as u64,
                    Value::Queue(q) => q.0.len() as u64,
                    Value::HeapMin(h) => h.0.len() as u64,
                    Value::HeapMax(h) => h.0.len() as u64,
                    Value::Sparse(s) => s.len() as u64,
                    Value::BTree(b) => b.len() as u64,
                    Value::Slice(s) => s.len() as u64,
                    Value::Deque(d) => d.0.len() as u64,
                    Value::Bitset(b) => b.len as u64, // bits, not bytes
                    Value::Bitvec(bv) => bv.bit_len,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), Value::Uint32(len as u32));
                Ok(Action::Continue)
            }

            // ── Wrappers ─────────────────────────────────────────────────────
            Opcode::Some_ => {
                let val = read_val!(get_op!(1));
                write_val!(get_op!(0), Value::Option(Box::new(FasmOption::Some(val))));
                Ok(Action::Continue)
            }
            Opcode::IsSome => {
                let opt = read_val!(get_op!(0));
                let is = matches!(opt, Value::Option(ref o) if matches!(o.as_ref(), FasmOption::Some(_)));
                write_val!(get_op!(1), Value::Bool(is));
                Ok(Action::Continue)
            }
            Opcode::Unwrap => {
                let opt = read_val!(get_op!(0));
                let inner = match opt {
                    Value::Option(o) => match *o {
                        FasmOption::Some(v) => v,
                        FasmOption::None => return Err(Fault::UnwrapFault),
                    },
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), inner);
                Ok(Action::Continue)
            }
            Opcode::Ok_ => {
                let val = read_val!(get_op!(1));
                write_val!(get_op!(0), Value::Result(Box::new(FasmResult::Ok(val))));
                Ok(Action::Continue)
            }
            Opcode::Err_ => {
                let code = as_u32(&read_val!(get_op!(1)))?;
                write_val!(get_op!(0), Value::Result(Box::new(FasmResult::Err(code))));
                Ok(Action::Continue)
            }
            Opcode::IsOk => {
                let res = read_val!(get_op!(0));
                let is =
                    matches!(res, Value::Result(ref r) if matches!(r.as_ref(), FasmResult::Ok(_)));
                write_val!(get_op!(1), Value::Bool(is));
                Ok(Action::Continue)
            }
            Opcode::UnwrapOk => {
                let res = read_val!(get_op!(0));
                let inner = match res {
                    Value::Result(r) => match *r {
                        FasmResult::Ok(v) => v,
                        FasmResult::Err(_) => return Err(Fault::UnwrapFault),
                    },
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), inner);
                Ok(Action::Continue)
            }
            Opcode::UnwrapErr => {
                let res = read_val!(get_op!(0));
                let code = match res {
                    Value::Result(r) => match *r {
                        FasmResult::Err(c) => c,
                        FasmResult::Ok(_) => return Err(Fault::UnwrapFault),
                    },
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), Value::Uint32(code));
                Ok(Action::Continue)
            }

            // ── Cast ─────────────────────────────────────────────────────────
            Opcode::Cast => {
                let val = read_val!(get_op!(0));
                let target_type = if let Operand::Type(t) = get_op!(1) {
                    *t
                } else {
                    return Err(Fault::TypeMismatch);
                };
                let result = cast_value(val, target_type)?;
                write_val!(get_op!(2), result);
                Ok(Action::Continue)
            }

            // ── Error handling ────────────────────────────────────────────────
            Opcode::Try => {
                let catch_ip = label_target(get_op!(0))?;
                let end_ip = label_target(get_op!(1))?;
                let frame_snap = self.call_stack.last().unwrap().frame.snapshot();
                let global_snap = self.globals.snapshot();
                self.call_stack.last_mut().unwrap().try_guard = Some(TryGuard {
                    catch_ip,
                    end_ip,
                    frame_snap,
                    global_snap,
                });
                Ok(Action::Continue)
            }
            Opcode::Catch => {
                // Normal execution: skip the CATCH block by jumping to ENDTRY
                let guard = self.call_stack.last().unwrap().try_guard.as_ref();
                let end = guard
                    .map(|g| g.end_ip)
                    .unwrap_or(self.call_stack.last().unwrap().ip);
                self.call_stack.last_mut().unwrap().try_guard = None;
                Ok(Action::Jump(end))
            }
            Opcode::EndTry => {
                self.call_stack.last_mut().unwrap().try_guard = None;
                Ok(Action::Continue)
            }

            // ── DEQUE extensions ─────────────────────────────────────────────
            Opcode::Prepend => {
                let val = read_val!(get_op!(1));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Deque(d) => d.0.push_front(val),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::PopBack => {
                let val = {
                    let coll = self.write_target_mut(get_op!(0))?;
                    match coll {
                        Value::Deque(d) => d.0.pop_back().ok_or(Fault::IndexOutOfBounds)?,
                        _ => return Err(Fault::TypeMismatch),
                    }
                };
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }
            Opcode::PeekBack => {
                let coll = read_val!(get_op!(0));
                let val = match &coll {
                    Value::Deque(d) => d.0.back().cloned().ok_or(Fault::IndexOutOfBounds)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), val);
                Ok(Action::Continue)
            }

            // ── VEC native operations ─────────────────────────────────────────
            Opcode::VecSort => {
                // In-place unstable numeric sort ascending. Non-numeric elements sort as 0.
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Vec(v) => v.0.sort_unstable_by(|a, b| {
                        let ai = numeric_as_i64(a).unwrap_or(0);
                        let bi = numeric_as_i64(b).unwrap_or(0);
                        ai.cmp(&bi)
                    }),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::VecFilter => {
                // VEC_FILTER vec, op_byte, threshold, target
                // op_byte: 0=LT, 1=EQ, 2=GT
                let src = read_val!(get_op!(0));
                let op_byte = as_u32(&read_val!(get_op!(1)))? as u8;
                let threshold = read_val!(get_op!(2));
                let result = match &src {
                    Value::Vec(v) => {
                        let filtered: Vec<Value> =
                            v.0.iter()
                                .filter(|elem| match op_byte {
                                    0 => elem.cmp_lt(&threshold).unwrap_or(false),
                                    1 => elem.eq_val(&threshold),
                                    2 => elem.cmp_gt(&threshold).unwrap_or(false),
                                    _ => false,
                                })
                                .cloned()
                                .collect();
                        Value::Vec(FasmVec(filtered))
                    }
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(3), result);
                Ok(Action::Continue)
            }
            Opcode::VecMerge => {
                // Merge two sorted VECs into a new sorted VEC (O(n+m)).
                let a = read_val!(get_op!(0));
                let b = read_val!(get_op!(1));
                let result = match (&a, &b) {
                    (Value::Vec(va), Value::Vec(vb)) => {
                        let mut merged = Vec::with_capacity(va.0.len() + vb.0.len());
                        let (mut ai, mut bi) = (0, 0);
                        while ai < va.0.len() && bi < vb.0.len() {
                            let av = numeric_as_i64(&va.0[ai]).unwrap_or(0);
                            let bv = numeric_as_i64(&vb.0[bi]).unwrap_or(0);
                            if av <= bv {
                                merged.push(va.0[ai].clone());
                                ai += 1;
                            } else {
                                merged.push(vb.0[bi].clone());
                                bi += 1;
                            }
                        }
                        merged.extend_from_slice(&va.0[ai..]);
                        merged.extend_from_slice(&vb.0[bi..]);
                        Value::Vec(FasmVec(merged))
                    }
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), result);
                Ok(Action::Continue)
            }
            Opcode::VecSlice => {
                // VEC_SLICE vec, start, len, target — produces a SLICE (copied sub-range).
                let src = read_val!(get_op!(0));
                let start = as_u32(&read_val!(get_op!(1)))? as usize;
                let len = as_u32(&read_val!(get_op!(2)))? as usize;
                let result = match &src {
                    Value::Vec(v) => {
                        let end = (start + len).min(v.0.len());
                        let s = if start <= end {
                            v.0[start..end].to_vec()
                        } else {
                            vec![]
                        };
                        Value::Slice(FasmSlice(s))
                    }
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(3), result);
                Ok(Action::Continue)
            }

            // ── SPARSE operations ─────────────────────────────────────────────
            Opcode::SparseGet => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = read_val!(get_op!(0));
                let val = match &coll {
                    Value::Sparse(s) => s.get(key).cloned().ok_or(Fault::FieldNotFound)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), val);
                Ok(Action::Continue)
            }
            Opcode::SparseSet => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let val = read_val!(get_op!(2));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Sparse(s) => s.insert(key, val),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::SparseDel => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Sparse(s) => s.remove(key),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::SparseHas => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = read_val!(get_op!(0));
                let has = match &coll {
                    Value::Sparse(s) => s.contains_key(key),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), Value::Bool(has));
                Ok(Action::Continue)
            }

            // ── BTREE operations ──────────────────────────────────────────────
            Opcode::BTreeGet => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = read_val!(get_op!(0));
                let val = match &coll {
                    Value::BTree(b) => b.get(key).cloned().ok_or(Fault::FieldNotFound)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), val);
                Ok(Action::Continue)
            }
            Opcode::BTreeSet => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let val = read_val!(get_op!(2));
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::BTree(b) => b.insert(key, val),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BTreeDel => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::BTree(b) => b.remove(key),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BTreeHas => {
                let key = as_u32(&read_val!(get_op!(1)))?;
                let coll = read_val!(get_op!(0));
                let has = match &coll {
                    Value::BTree(b) => b.contains_key(key),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), Value::Bool(has));
                Ok(Action::Continue)
            }
            Opcode::BTreeMin => {
                let coll = read_val!(get_op!(0));
                let key = match &coll {
                    Value::BTree(b) => b.min_key().ok_or(Fault::IndexOutOfBounds)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), Value::Uint32(key));
                Ok(Action::Continue)
            }
            Opcode::BTreeMax => {
                let coll = read_val!(get_op!(0));
                let key = match &coll {
                    Value::BTree(b) => b.max_key().ok_or(Fault::IndexOutOfBounds)?,
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), Value::Uint32(key));
                Ok(Action::Continue)
            }

            // ── BITSET operations ─────────────────────────────────────────────
            Opcode::BitSet_ => {
                let idx = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitset(b) => b.set_bit(idx),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitClr => {
                let idx = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitset(b) => b.clr_bit(idx),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitGet => {
                let idx = as_u32(&read_val!(get_op!(1)))?;
                let coll = read_val!(get_op!(0));
                let bit = match &coll {
                    Value::Bitset(b) => b.get_bit(idx),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(2), Value::Bool(bit));
                Ok(Action::Continue)
            }
            Opcode::BitFlip => {
                let idx = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitset(b) => b.flip_bit(idx),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitCount => {
                let coll = read_val!(get_op!(0));
                let count = match &coll {
                    Value::Bitset(b) => b.popcount(),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(1), Value::Uint32(count));
                Ok(Action::Continue)
            }
            Opcode::BitAnd => {
                let src_val = read_val!(get_op!(1));
                let dst = self.write_target_mut(get_op!(0))?;
                match (dst, &src_val) {
                    (Value::Bitset(d), Value::Bitset(s)) => {
                        let min = d.bytes.len().min(s.bytes.len());
                        for i in 0..min {
                            d.bytes[i] &= s.bytes[i];
                        }
                        for i in min..d.bytes.len() {
                            d.bytes[i] = 0;
                        }
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitOr => {
                let src_val = read_val!(get_op!(1));
                let dst = self.write_target_mut(get_op!(0))?;
                match (dst, &src_val) {
                    (Value::Bitset(d), Value::Bitset(s)) => {
                        if s.bytes.len() > d.bytes.len() {
                            d.bytes.resize(s.bytes.len(), 0);
                        }
                        for i in 0..s.bytes.len() {
                            d.bytes[i] |= s.bytes[i];
                        }
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitXor => {
                let src_val = read_val!(get_op!(1));
                let dst = self.write_target_mut(get_op!(0))?;
                match (dst, &src_val) {
                    (Value::Bitset(d), Value::Bitset(s)) => {
                        if s.bytes.len() > d.bytes.len() {
                            d.bytes.resize(s.bytes.len(), 0);
                        }
                        for i in 0..s.bytes.len() {
                            d.bytes[i] ^= s.bytes[i];
                        }
                    }
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitGrow => {
                let n_bits = as_u32(&read_val!(get_op!(1)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitset(b) => b.grow_to(b.len + n_bits),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }

            // ── BITVEC operations ─────────────────────────────────────────────
            Opcode::BitvecRead => {
                // BITVEC_READ bv, bit_start, bit_len, target
                let bit_start = as_u64(&read_val!(get_op!(1)))?;
                let bit_len = as_u32(&read_val!(get_op!(2)))?;
                let coll = read_val!(get_op!(0));
                let result = match &coll {
                    Value::Bitvec(bv) => bv.read_bits(bit_start, bit_len),
                    _ => return Err(Fault::TypeMismatch),
                };
                write_val!(get_op!(3), Value::Uint64(result));
                Ok(Action::Continue)
            }
            Opcode::BitvecWrite => {
                // BITVEC_WRITE bv, bit_start, bit_len, value
                let bit_start = as_u64(&read_val!(get_op!(1)))?;
                let bit_len = as_u32(&read_val!(get_op!(2)))?;
                let value = as_u64(&read_val!(get_op!(3)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitvec(bv) => bv.write_bits(bit_start, bit_len, value),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
            Opcode::BitvecPush => {
                // BITVEC_PUSH bv, value, bit_len
                let value = as_u64(&read_val!(get_op!(1)))?;
                let bit_len = as_u32(&read_val!(get_op!(2)))?;
                let coll = self.write_target_mut(get_op!(0))?;
                match coll {
                    Value::Bitvec(bv) => bv.push_bits(value, bit_len),
                    _ => return Err(Fault::TypeMismatch),
                }
                Ok(Action::Continue)
            }
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn read_operand(&self, op: &Operand) -> Result<Value, Fault> {
        match op {
            Operand::Slot(s) => self.read_slot(s),
            Operand::Imm(imm) => Ok(imm_to_value(imm)),
            _ => Err(Fault::TypeMismatch),
        }
    }

    fn read_slot(&self, s: &SlotRef) -> Result<Value, Fault> {
        match s {
            SlotRef::Local(idx) => {
                let cf = self.call_stack.last().unwrap();
                cf.frame.get(*idx).cloned().ok_or(Fault::UndeclaredSlot)
            }
            SlotRef::Tmp(idx) => {
                let cf = self.call_stack.last().unwrap();
                let f = cf.tmp_frames.last().ok_or(Fault::IndexOutOfBounds)?; // no tmp block active
                if *idx > 15 {
                    return Err(Fault::IndexOutOfBounds);
                }
                f.slots[*idx as usize].clone().ok_or(Fault::UndeclaredSlot) // Tmp slots must have been written to
            }
            SlotRef::Global(idx) => self
                .globals
                .get(*idx as u32)
                .cloned()
                .ok_or(Fault::UndeclaredSlot),
            SlotRef::DerefLocal(idx) => {
                let cf = self.call_stack.last().unwrap();
                let ref_val = cf.frame.get(*idx).ok_or(Fault::UndeclaredSlot)?;
                self.deref_value(ref_val)
            }
            SlotRef::DerefTmp(idx) => {
                let cf = self.call_stack.last().unwrap();
                let f = cf.tmp_frames.last().ok_or(Fault::IndexOutOfBounds)?;
                if *idx > 15 {
                    return Err(Fault::IndexOutOfBounds);
                }
                let ref_val = f.slots[*idx as usize]
                    .as_ref()
                    .ok_or(Fault::UndeclaredSlot)?;
                self.deref_value(ref_val)
            }
            SlotRef::DerefGlobal(idx) => {
                let ref_val = self.globals.get(*idx as u32).ok_or(Fault::UndeclaredSlot)?;
                self.deref_value(ref_val)
            }
            SlotRef::BuiltIn(b) => {
                let cf = self.call_stack.last().unwrap();
                Ok(match b {
                    BuiltIn::Args => cf.args.clone(),
                    BuiltIn::Ret => cf.ret_val.clone(),
                    BuiltIn::FaultIndex => Value::Uint32(cf.ip as u32),
                    BuiltIn::FaultCode => cf.ret_val.clone(), // fault code stored in ret
                })
            }
        }
    }

    fn deref_value(&self, ref_val: &Value) -> Result<Value, Fault> {
        match ref_val {
            Value::RefMut(is_global, idx) | Value::RefImm(is_global, idx) => {
                if *is_global {
                    self.globals.get(*idx).cloned().ok_or(Fault::NullDeref)
                } else {
                    let cf = self.call_stack.last().unwrap();
                    cf.frame.get(*idx as u8).cloned().ok_or(Fault::NullDeref)
                }
            }
            Value::Null => Err(Fault::NullDeref),
            _ => Ok(ref_val.clone()),
        }
    }

    fn write_operand(&mut self, op: &Operand, val: Value) -> Result<(), Fault> {
        match op {
            Operand::Slot(s) => self.write_slot(s, val),
            _ => Err(Fault::TypeMismatch),
        }
    }

    fn write_slot(&mut self, s: &SlotRef, val: Value) -> Result<(), Fault> {
        match s {
            SlotRef::Local(idx) => {
                self.call_stack.last_mut().unwrap().frame.set(*idx, val);
                Ok(())
            }
            SlotRef::Tmp(idx) => {
                let cf = self.call_stack.last_mut().unwrap();
                let f = cf.tmp_frames.last_mut().ok_or(Fault::IndexOutOfBounds)?;
                if *idx > 15 {
                    return Err(Fault::IndexOutOfBounds);
                }
                f.slots[*idx as usize] = Some(val);
                Ok(())
            }
            SlotRef::Global(idx) => {
                self.globals.set(*idx as u32, val);
                Ok(())
            }
            SlotRef::DerefLocal(idx) => {
                let ref_val = {
                    let cf = self.call_stack.last().unwrap();
                    cf.frame.get(*idx).cloned().ok_or(Fault::UndeclaredSlot)?
                };
                self.deref_write(ref_val, val)
            }
            SlotRef::DerefTmp(idx) => {
                let ref_val = {
                    let cf = self.call_stack.last().unwrap();
                    let f = cf.tmp_frames.last().ok_or(Fault::IndexOutOfBounds)?;
                    if *idx > 15 {
                        return Err(Fault::IndexOutOfBounds);
                    }
                    f.slots[*idx as usize]
                        .clone()
                        .ok_or(Fault::UndeclaredSlot)?
                };
                self.deref_write(ref_val, val)
            }
            SlotRef::DerefGlobal(idx) => {
                let ref_val = self
                    .globals
                    .get(*idx as u32)
                    .cloned()
                    .ok_or(Fault::UndeclaredSlot)?;
                self.deref_write(ref_val, val)
            }
            SlotRef::BuiltIn(b) => {
                let cf = self.call_stack.last_mut().unwrap();
                match b {
                    BuiltIn::Ret => {
                        cf.ret_val = val;
                        Ok(())
                    }
                    _ => Err(Fault::WriteAccessViolation),
                }
            }
        }
    }

    fn deref_write(&mut self, ref_val: Value, val: Value) -> Result<(), Fault> {
        match ref_val {
            Value::RefMut(is_global, idx) => {
                if is_global {
                    self.globals.set(idx, val);
                } else {
                    self.call_stack
                        .last_mut()
                        .unwrap()
                        .frame
                        .set(idx as u8, val);
                }
                Ok(())
            }
            Value::RefImm(_, _) => Err(Fault::WriteAccessViolation),
            Value::Null => Err(Fault::NullDeref),
            _ => Err(Fault::TypeMismatch),
        }
    }

    fn write_target_mut(&mut self, op: &Operand) -> Result<&mut Value, Fault> {
        match op {
            Operand::Slot(SlotRef::Local(idx)) => {
                let cf = self.call_stack.last_mut().unwrap();
                cf.frame.get_mut(*idx).ok_or(Fault::UndeclaredSlot)
            }
            Operand::Slot(SlotRef::Tmp(idx)) => {
                let cf = self.call_stack.last_mut().unwrap();
                let f = cf.tmp_frames.last_mut().ok_or(Fault::IndexOutOfBounds)?;
                if *idx > 15 {
                    return Err(Fault::IndexOutOfBounds);
                }
                f.slots[*idx as usize].as_mut().ok_or(Fault::UndeclaredSlot)
            }
            Operand::Slot(SlotRef::Global(idx)) => self
                .globals
                .get_mut(*idx as u32)
                .ok_or(Fault::UndeclaredSlot),
            _ => Err(Fault::TypeMismatch),
        }
    }

    fn release_operand(&mut self, op: &Operand) {
        match op {
            Operand::Slot(SlotRef::Local(idx)) => {
                if let Some(cf) = self.call_stack.last_mut() {
                    cf.frame.remove(*idx);
                }
            }
            Operand::Slot(SlotRef::Tmp(idx)) => {
                if let Some(cf) = self.call_stack.last_mut() {
                    if let Some(f) = cf.tmp_frames.last_mut() {
                        if *idx <= 15 {
                            f.slots[*idx as usize] = None;
                        }
                    }
                }
            }
            Operand::Slot(SlotRef::Global(idx)) => {
                self.globals.remove(*idx as u32);
            }
            _ => {}
        }
    }

    fn slot_address(&self, op: &Operand) -> (bool, u32) {
        match op {
            Operand::Slot(SlotRef::Local(i)) => (false, *i as u32),
            Operand::Slot(SlotRef::Global(i)) => (true, *i as u32),
            _ => (false, 0),
        }
    }

    fn imm_operand(&self, op: &Operand) -> Result<Value, Fault> {
        match op {
            Operand::Imm(imm) => Ok(imm_to_value(imm)),
            Operand::Slot(_) => self.read_operand(op),
            _ => Err(Fault::TypeMismatch),
        }
    }

    fn read_key_operand(&self, op: &Operand) -> Result<u32, Fault> {
        match op {
            Operand::Key(k) => Ok(*k),
            Operand::Imm(Immediate::Uint32(k)) => Ok(*k),
            Operand::Slot(_) => {
                let v = self.read_operand(op)?;
                as_u32(&v)
            }
            _ => Err(Fault::TypeMismatch),
        }
    }
}

// ─── Free helpers ─────────────────────────────────────────────────────────────

enum Action {
    Continue,
    Jump(usize),
    CallFunc(String, Value),
    TailCall(String, Value),
    Return(Value),
    Halt,
}

fn label_target(op: &Operand) -> Result<usize, Fault> {
    match op {
        Operand::LabelTarget(t) => Ok(*t as usize),
        _ => Err(Fault::TypeMismatch),
    }
}

fn syscall_id(op: &Operand) -> Result<i32, Fault> {
    match op {
        Operand::SyscallId(id) => Ok(*id),
        Operand::Imm(Immediate::Int32(id)) => Ok(*id),
        _ => Err(Fault::TypeMismatch),
    }
}

fn as_u32(v: &Value) -> Result<u32, Fault> {
    Ok(match v {
        Value::Uint8(n) => *n as u32,
        Value::Uint16(n) => *n as u32,
        Value::Uint32(n) => *n,
        Value::Uint64(n) => *n as u32,
        Value::Int8(n) => *n as u32,
        Value::Int16(n) => *n as u32,
        Value::Int32(n) => *n as u32,
        Value::Int64(n) => *n as u32,
        _ => return Err(Fault::TypeMismatch),
    })
}

fn as_u64(v: &Value) -> Result<u64, Fault> {
    numeric_as_u64(v).ok_or(Fault::TypeMismatch)
}

fn imm_to_value(imm: &Immediate) -> Value {
    match imm {
        Immediate::Bool(b) => Value::Bool(*b),
        Immediate::Int8(n) => Value::Int8(*n),
        Immediate::Int16(n) => Value::Int16(*n),
        Immediate::Int32(n) => Value::Int32(*n),
        Immediate::Int64(n) => Value::Int64(*n),
        Immediate::Uint8(n) => Value::Uint8(*n),
        Immediate::Uint16(n) => Value::Uint16(*n),
        Immediate::Uint32(n) => Value::Uint32(*n),
        Immediate::Uint64(n) => Value::Uint64(*n),
        Immediate::Float32(f) => Value::Float32(*f),
        Immediate::Float64(f) => Value::Float64(*f),
        Immediate::Null => Value::Null,
        // String literals expand to VEC<UINT8> using their UTF-8 byte representation.
        // SYSCALL 0 (PRINT) and SYSCALL 1 (PRINT_VEC) both handle VEC<UINT8> as text.
        Immediate::Str(s) => {
            let bytes = s.bytes().map(Value::Uint8).collect();
            Value::Vec(FasmVec(bytes))
        }
    }
}

fn imm_to_value_for_type(t: FasmType, op: &Operand) -> Value {
    match op {
        // NULL init means use the type's default zero value
        Operand::Imm(Immediate::Null) => default_for_type(t),
        Operand::Type(FasmType::Null) => default_for_type(t),
        Operand::Imm(imm) => imm_to_value(imm),
        _ => default_for_type(t),
    }
}

fn default_for_type(t: FasmType) -> Value {
    match t {
        FasmType::Bool => Value::Bool(false),
        FasmType::Int8 => Value::Int8(0),
        FasmType::Int16 => Value::Int16(0),
        FasmType::Int32 => Value::Int32(0),
        FasmType::Int64 => Value::Int64(0),
        FasmType::Uint8 => Value::Uint8(0),
        FasmType::Uint16 => Value::Uint16(0),
        FasmType::Uint32 => Value::Uint32(0),
        FasmType::Uint64 => Value::Uint64(0),
        FasmType::Float32 => Value::Float32(0.0),
        FasmType::Float64 => Value::Float64(0.0),
        FasmType::RefMut => Value::RefMut(false, 0),
        FasmType::RefImm => Value::RefImm(false, 0),
        FasmType::Vec => Value::Vec(FasmVec::default()),
        FasmType::Struct => Value::Struct(FasmStruct::default()),
        FasmType::Stack => Value::Stack(FasmStack::default()),
        FasmType::Queue => Value::Queue(FasmQueue::default()),
        FasmType::HeapMin => Value::HeapMin(FasmHeapMin::default()),
        FasmType::HeapMax => Value::HeapMax(FasmHeapMax::default()),
        FasmType::Sparse => Value::Sparse(FasmSparse::default()),
        FasmType::BTree => Value::BTree(FasmBTree::default()),
        FasmType::Slice => Value::Slice(FasmSlice::default()),
        FasmType::Deque => Value::Deque(FasmDeque::default()),
        FasmType::Bitset => Value::Bitset(FasmBitset::default()),
        FasmType::Bitvec => Value::Bitvec(FasmBitvec::default()),
        FasmType::Option => Value::Option(Box::new(FasmOption::None)),
        FasmType::Result => Value::Result(Box::new(FasmResult::Err(0))),
        FasmType::Future => Value::Future(None),
        FasmType::Null => Value::Null,
    }
}

fn cast_value(val: Value, target: FasmType) -> Result<Value, Fault> {
    // Numeric conversions
    let as_i64: Option<i64> = match &val {
        Value::Bool(b) => Some(*b as i64),
        Value::Int8(n) => Some(*n as i64),
        Value::Int16(n) => Some(*n as i64),
        Value::Int32(n) => Some(*n as i64),
        Value::Int64(n) => Some(*n),
        Value::Uint8(n) => Some(*n as i64),
        Value::Uint16(n) => Some(*n as i64),
        Value::Uint32(n) => Some(*n as i64),
        Value::Uint64(n) => Some(*n as i64),
        Value::Float32(f) => Some(*f as i64),
        Value::Float64(f) => Some(*f as i64),
        _ => None,
    };
    let as_f64: Option<f64> = match &val {
        Value::Float32(f) => Some(*f as f64),
        Value::Float64(f) => Some(*f),
        _ => as_i64.map(|n| n as f64),
    };
    match target {
        FasmType::Bool => as_i64
            .map(|n| Value::Bool(n != 0))
            .ok_or(Fault::TypeMismatch),
        FasmType::Int8 => as_i64
            .map(|n| Value::Int8(n as i8))
            .ok_or(Fault::TypeMismatch),
        FasmType::Int16 => as_i64
            .map(|n| Value::Int16(n as i16))
            .ok_or(Fault::TypeMismatch),
        FasmType::Int32 => as_i64
            .map(|n| Value::Int32(n as i32))
            .ok_or(Fault::TypeMismatch),
        FasmType::Int64 => as_i64.map(Value::Int64).ok_or(Fault::TypeMismatch),
        FasmType::Uint8 => as_i64
            .map(|n| Value::Uint8(n as u8))
            .ok_or(Fault::TypeMismatch),
        FasmType::Uint16 => as_i64
            .map(|n| Value::Uint16(n as u16))
            .ok_or(Fault::TypeMismatch),
        FasmType::Uint32 => as_i64
            .map(|n| Value::Uint32(n as u32))
            .ok_or(Fault::TypeMismatch),
        FasmType::Uint64 => as_i64
            .map(|n| Value::Uint64(n as u64))
            .ok_or(Fault::TypeMismatch),
        FasmType::Float32 => as_f64
            .map(|f| Value::Float32(f as f32))
            .ok_or(Fault::TypeMismatch),
        FasmType::Float64 => as_f64.map(Value::Float64).ok_or(Fault::TypeMismatch),
        _ => Err(Fault::TypeMismatch),
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}
