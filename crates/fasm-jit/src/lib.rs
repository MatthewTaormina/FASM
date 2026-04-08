//! # fasm-jit
//!
//! A Cranelift-based JIT compiler for FASM numeric functions.
//!
//! ## What gets JIT-compiled
//! A function is eligible when all its local slots use numeric types
//! (Bool, Int*, Uint*, Float*) or are STRUCT-accumulators used only for
//! argument-passing to tail-recursive self-calls.  Syscalls, collection
//! operations, TRY/CATCH, and non-self CALL instructions cause the function
//! to fall back to the bytecode interpreter.
//!
//! ## Integration
//! ```rust,ignore
//! use fasm_jit::FasmJit;
//! use fasm_vm::Executor;
//!
//! let program = fasm_compiler::compile_source(source).unwrap();
//! let mut executor = Executor::new();
//! if let Some(jit) = FasmJit::compile(&program) {
//!     executor.set_jit(std::sync::Arc::new(jit));
//! }
//! executor.run(&program).unwrap();
//! ```

pub mod analyze;
pub mod codegen;

pub use analyze::{analyze_program, JitFnInfo, JitType};
pub use codegen::{compile_program, pack_args, unpack_ret, JitCache, JitEntry};

use fasm_bytecode::Program;
use fasm_vm::{JitDispatcher, Value};

// ─── Public high-level API ────────────────────────────────────────────────────

/// A compiled JIT cache that implements [`fasm_vm::JitDispatcher`].
///
/// Create with [`FasmJit::compile`] and attach to an [`fasm_vm::Executor`]
/// with [`fasm_vm::Executor::set_jit`].
pub struct FasmJit {
    cache: JitCache,
}

impl FasmJit {
    /// Compile all eligible functions in `program`.
    /// Returns `None` if no functions are eligible or the host ISA is unsupported.
    pub fn compile(program: &Program) -> Option<Self> {
        let eligible = analyze_program(program);
        let cache = compile_program(program, &eligible)?;
        Some(FasmJit { cache })
    }
}

impl JitDispatcher for FasmJit {
    fn dispatch(&self, func_idx: usize, args: &Value) -> Option<Value> {
        let entry = self.cache.entries.get(&func_idx)?;
        // SAFETY: `entry` was produced by `compile_program` for this `cache`.
        // The cache is owned by `self` and lives as long as this dispatcher.
        unsafe { codegen::call_jit(entry, args) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use fasm_compiler::compile_source;
    use fasm_vm::value::FasmStruct;
    use fasm_vm::Value;

    fn fib_src() -> &'static str {
        r#"
DEFINE ARG_N, 0
DEFINE ARG_A, 1
DEFINE ARG_B, 2

FUNC Fibonacci
    PARAM ARG_N, INT32, n, REQUIRED
    PARAM ARG_A, INT32, a, REQUIRED
    PARAM ARG_B, INT32, b, REQUIRED

    LOCAL 0, BOOL,  is_base_0
    LOCAL 1, BOOL,  is_base_1
    LOCAL 2, INT32, n_val
    LOCAL 3, INT32, a_val
    LOCAL 4, INT32, b_val
    LOCAL 5, INT32, next_n
    LOCAL 6, INT32, next_b
    LOCAL 7, STRUCT, next_args

    GET_FIELD $args, ARG_N, n_val
    GET_FIELD $args, ARG_A, a_val
    GET_FIELD $args, ARG_B, b_val

    EQ n_val, 0, is_base_0
    JNZ is_base_0, Base0

    EQ n_val, 1, is_base_1
    JNZ is_base_1, Base1

    SUB n_val, 1, next_n
    ADD a_val, b_val, next_b

    RESERVE 7, STRUCT, NULL
    SET_FIELD next_args, ARG_N, next_n
    SET_FIELD next_args, ARG_A, b_val
    SET_FIELD next_args, ARG_B, next_b
    TAIL_CALL Fibonacci, next_args
    RET $ret

    LABEL Base0
    RET a_val

    LABEL Base1
    RET b_val
ENDF

FUNC Main
    LOCAL 0, INT32, answer
    LOCAL 1, STRUCT, args

    RESERVE 1, STRUCT, NULL
    SET_FIELD args, ARG_N, 10
    SET_FIELD args, ARG_A, 0
    SET_FIELD args, ARG_B, 1
    CALL Fibonacci, args
    MOV $ret, answer
    RET answer
ENDF
"#
    }

    fn run_fib_jit(n: i32) -> Option<Value> {
        let program = compile_source(fib_src()).expect("compile");
        let eligible = analyze_program(&program);
        let fib_idx = program.get_function_index("Fibonacci")?;
        assert!(eligible.contains_key(&fib_idx), "Fibonacci must be JIT-eligible");
        let cache = compile_program(&program, &eligible)?;
        let entry = cache.entries.get(&fib_idx)?;
        let mut s = FasmStruct(Vec::new());
        s.insert(0, Value::Int32(n));
        s.insert(1, Value::Int32(0));
        s.insert(2, Value::Int32(1));
        unsafe { codegen::call_jit(entry, &Value::Struct(s)) }
    }

    #[test]
    fn test_fibonacci_jit_base_cases() {
        assert_eq!(run_fib_jit(0), Some(Value::Int32(0)));
        assert_eq!(run_fib_jit(1), Some(Value::Int32(1)));
    }

    #[test]
    fn test_fibonacci_jit_fib10() {
        assert_eq!(run_fib_jit(10), Some(Value::Int32(55)));
    }

    #[test]
    fn test_fibonacci_jit_fib30() {
        assert_eq!(run_fib_jit(30), Some(Value::Int32(832040)));
    }

    #[test]
    fn test_analyze_fibonacci_eligible() {
        let prog = compile_source(fib_src()).expect("compile");
        let eligible = analyze_program(&prog);
        let fib_idx = prog.get_function_index("Fibonacci").unwrap();
        assert!(eligible.contains_key(&fib_idx), "Fibonacci must be JIT-eligible");
        let info = &eligible[&fib_idx];
        assert_eq!(info.params.len(), 3);
        assert_eq!(info.ret_type, JitType::I32);
    }

    #[test]
    fn test_analyze_syscall_ineligible() {
        let src = r#"
FUNC Main
    LOCAL 0, INT32, x
    LOCAL 1, STRUCT, args
    RESERVE 0, INT32, 42
    RESERVE 1, STRUCT, NULL
    SYSCALL 0, args
    RET x
ENDF
"#;
        let prog = compile_source(src).expect("compile");
        let eligible = analyze_program(&prog);
        let main_idx = prog.get_function_index("Main").unwrap();
        assert!(!eligible.contains_key(&main_idx), "Syscall makes Main ineligible");
    }

    #[test]
    fn test_analyze_vec_ineligible() {
        let src = r#"
FUNC Main
    LOCAL 0, VEC, v
    RESERVE 0, VEC, NULL
    RET
ENDF
"#;
        let prog = compile_source(src).expect("compile");
        let eligible = analyze_program(&prog);
        let main_idx = prog.get_function_index("Main").unwrap();
        assert!(!eligible.contains_key(&main_idx));
    }

    #[test]
    fn test_executor_jit_integration() {
        let src = fib_src();
        let program = compile_source(src).expect("compile");

        let mut exec_interp = fasm_vm::Executor::new();
        let interp_result = exec_interp.run(&program).expect("interpreter run");

        let mut exec_jit = fasm_vm::Executor::new();
        if let Some(jit) = FasmJit::compile(&program) {
            exec_jit.set_jit(std::sync::Arc::new(jit));
        }
        let jit_result = exec_jit.run(&program).expect("jit run");

        assert_eq!(interp_result, jit_result,
            "JIT and interpreter must produce the same result");
    }
}
