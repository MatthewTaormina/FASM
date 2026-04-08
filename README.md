# FASM — Function-first Assembly

A sandboxed, typed assembly language that compiles to compact `.fasmc` bytecode and runs on a high-performance Rust VM. FASM is designed as an embeddable execution layer: deterministic, memory-safe, and extensible through native syscall handlers and IPC sidecar plugins.

---

## Quick Start

**Prerequisites**: Rust toolchain (`cargo`)

```powershell
# Build everything
cargo build --release

# Run a FASM source file (compile + execute in one step)
.\target\release\fasm.exe run examples\fibonacci.fasm

# Compile to bytecode, then execute the bytecode
.\target\release\fasm.exe compile examples\fibonacci.fasm -o out.fasmc
.\target\release\fasm.exe exec out.fasmc

# Static validation only (no execution)
.\target\release\fasm.exe check examples\fibonacci.fasm

# Microbenchmark the VM (pure execution, I/O suppressed)
.\target\release\fasm.exe bench out.fasmc 50000
```

### Install globally

```powershell
cargo install --path crates\fasm-cli
fasm run examples\fibonacci.fasm
```

---

## Workspace Layout

```
FASM/
├── FASM.md                    ← Full language specification
├── Cargo.toml                 ← Workspace manifest
├── benchmark.ps1              ← End-to-end cold-start benchmark script
├── crates/
│   ├── fasm-bytecode/         ← Instruction model, opcodes, binary encode/decode
│   ├── fasm-compiler/         ← Lexer → Parser → Validator → Emitter
│   ├── fasm-vm/               ← Runtime executor, memory, Value types, fault codes
│   ├── fasm-sandbox/          ← Isolated execution context, clock throttling, IPC sidecars
│   └── fasm-cli/              ← `fasm` CLI binary
│       └── src/bin/
│           └── fibbench_native.rs  ← Native Rust fib benchmark (fair VM comparison)
├── examples/
│   ├── fibonacci.fasm         ← Tail-call Fibonacci (fib(30) = 832040)
│   ├── calculator.fasm        ← Interactive CLI calculator (I/O, RESULT, error handling)
│   ├── calculator.c           ← C reference implementation of calculator
│   ├── calculator.js          ← Node.js reference implementation
│   ├── calculator.py          ← Python reference implementation
│   ├── fibbench.c             ← C benchmark (matches fibonacci.fasm algorithm + N)
│   ├── fibbench.js            ← Node.js fibonacci benchmark
│   ├── fibbench.py            ← Python fibonacci benchmark
│   ├── bench_sidecar.fasm     ← IPC sidecar throughput benchmark
│   ├── sidecar.fasm           ← Sidecar plugin usage example
│   ├── tco_test.fasm          ← Tail-call optimisation smoke test
│   ├── my_plugin.py           ← Example Python sidecar plugin
│   └── ping.py                ← IPC ping sidecar (used by bench_sidecar)
└── tools/
    └── disasm.rs              ← Debug disassembler utility (standalone)
```

---

## Language Overview

FASM (Function-first Assembly) is a typed, frame-based bytecode language with:

- **Typed slots** — every local and global slot has a declared type (`INT32`, `STRUCT`, `VEC`, …)
- **Struct-based calling convention** — all functions and syscalls pass a `STRUCT` argument
- **Rich collection types** — `VEC`, `STRUCT`, `STACK`, `QUEUE`, `HEAP_MIN`, `HEAP_MAX`, `SPARSE`, `BTREE`, `SLICE`, `DEQUE`, `BITSET`, `BITVEC`
- **Wrapper types** — `OPTION`, `RESULT`, `FUTURE`
- **References** — `REF_MUT` / `REF_IMM` with `&` dereference syntax
- **Transactional error handling** — `TRY` / `CATCH` / `ENDTRY` with automatic memory rollback
- **Cooperative async** — `ASYNC CALL` returns a `FUTURE`, `AWAIT` resolves it
- **Global register** — u32-indexed global slots shared across all function frames
- **Tail-call optimisation** — `TAIL_CALL` recycles the current frame with zero stack growth
- **Static validation** — invalid programs are rejected at compile time before execution

### Hello World

```fasm
FUNC Main
    LOCAL 0, STRUCT, args

    RESERVE 0, STRUCT, NULL
    SET_FIELD args, 0, "Hello, World!"
    SYSCALL 0, args             // PRINT — prints with newline
ENDF
```

### Fibonacci (tail-call, linear time)

```fasm
DEFINE ARG_N, 0
DEFINE ARG_A, 1
DEFINE ARG_B, 2

FUNC Fibonacci
    PARAM ARG_N, INT32, n, REQUIRED
    PARAM ARG_A, INT32, a, REQUIRED
    PARAM ARG_B, INT32, b, REQUIRED

    LOCAL 0, BOOL,   is_base_0
    LOCAL 1, BOOL,   is_base_1
    LOCAL 2, INT32,  n_val
    LOCAL 3, INT32,  a_val
    LOCAL 4, INT32,  b_val
    LOCAL 5, INT32,  next_n
    LOCAL 6, INT32,  next_b
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

    LABEL Base0
    RET a_val

    LABEL Base1
    RET b_val
ENDF

FUNC Main
    LOCAL 0, INT32,  answer
    LOCAL 1, STRUCT, args
    LOCAL 2, STRUCT, print_args

    RESERVE 1, STRUCT, NULL
    SET_FIELD args, ARG_N, 30
    SET_FIELD args, ARG_A, 0
    SET_FIELD args, ARG_B, 1
    CALL Fibonacci, args
    MOV $ret, answer

    RESERVE 2, STRUCT, NULL
    SET_FIELD print_args, 0, answer
    SYSCALL 0, print_args       // prints 832040
ENDF
```

---

## Instruction Set

| Category | Instructions |
|---|---|
| Memory | `RESERVE`, `RELEASE`, `TMP_BLOCK`, `END_TMP` |
| Data movement | `MOV`, `STORE`, `ADDR` |
| Arithmetic | `ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `NEG` |
| Comparison | `EQ`, `NEQ`, `LT`, `LTE`, `GT`, `GTE` |
| Bitwise | `AND`, `OR`, `XOR`, `NOT`, `SHL`, `SHR` |
| Control flow | `JMP`, `JZ`, `JNZ`, `CALL`, `TAIL_CALL`, `RET`, `HALT` |
| Async | `ASYNC CALL`, `ASYNC SYSCALL`, `AWAIT` |
| Syscall | `SYSCALL`, `ASYNC SYSCALL` |
| Vec / Stack / Queue | `PUSH`, `POP`, `ENQUEUE`, `DEQUEUE`, `PEEK`, `GET_IDX`, `SET_IDX`, `LEN`, `VEC_FILTER_GT`, `VEC_FILTER_LT`, `VEC_FILTER_EQ`, `VEC_MERGE_SORTED`, `VEC_SLICE`, `PREPEND`, `BITVEC_PUSH` |
| Struct / Adv Collections | `GET_FIELD`, `SET_FIELD`, `HAS_FIELD`, `DEL_FIELD`, `SPARSE_GET`, `SPARSE_SET`, `SPARSE_DEL`, `SPARSE_HAS`, `BTREE_GET`, `BTREE_SET`, `BTREE_DEL`, `BTREE_HAS`, `BTREE_MIN`, `BTREE_MAX`, `BITSET_SET`, `BITSET_CLEAR`, `BITSET_TOGGLE`, `BITSET_TEST` |
| Wrappers | `SOME`, `IS_SOME`, `UNWRAP`, `OK`, `ERR`, `IS_OK`, `UNWRAP_OK`, `UNWRAP_ERR` |
| Cast | `CAST` |
| Error handling | `TRY`, `CATCH`, `ENDTRY` |

---

## Built-in Syscalls

| ID | Name | Args (struct key → value) | Returns |
|---|---|---|---|
| `0` | `PRINT` | `0` → any value (appends newline) | `NULL` |
| `1` | `PRINT_VEC` | `0` → `VEC<UINT8>` (no newline) | `NULL` |
| `2` | `READ` | — | `VEC<UINT8>` (stdin line, whitespace trimmed) |
| `3` | `EXIT` | `0` → `INT32` exit code | terminates |
| `4` | `PARSE_INT` | `0` → `VEC<UINT8>` ASCII digits | `RESULT<INT32>` |

Custom syscalls can be registered at runtime by the host via `Sandbox::mount_syscall(id, handler)`.

---

## IPC Sidecar Plugins

FASM can offload syscalls to external processes over stdin/stdout JSON-RPC — no FFI required.

```powershell
.\target\release\fasm.exe run script.fasm --plugin 99:python:plugin.py
```

This binds Syscall `#99` to `python plugin.py`. The sidecar receives the call arguments as JSON and returns a JSON value. Multiple syscall IDs can share a single sidecar process:

```powershell
--plugin 10,11,12:python:plugin.py
```

---

## Benchmarks

Tested on a single machine, release build (`cargo build --release`).

### Fibonacci(30) — pure VM execution, 50,000 iterations

| Runtime | Algorithm | Time/iter |
|---|---|---|
| Native Rust (`fibbench_native`) | Iterative accumulator | ~0.07 µs |
| FASM VM (`fasm bench`) | Tail-call (`TAIL_CALL`) | ~74 µs |

The VM overhead is ~1000x vs native — expected for an interpreted bytecode VM.  
CPython is typically 50–100x slower than native C on equivalent workloads.

### Running the benchmark

```powershell
# Build the native reference
cargo build --release

# Compile FASM fibonacci to bytecode
.\target\release\fasm.exe compile examples\fibonacci.fasm -o fibonacci.fasmc

# Run the FASM VM benchmark (50,000 iterations, I/O suppressed)
.\target\release\fasm.exe bench fibonacci.fasmc 50000

# Run the native Rust reference benchmark
.\target\release\fibbench_native.exe
```

---

## Bytecode Format (`.fasmc`)

For a detailed bytecode-level format and binary encoding layout mapping specific components, please explicitly consult the detailed specification at **[BYTECODE.md](BYTECODE.md)**. The fundamental stream operates generally under:

```
[4 bytes]  magic: "FSMC"
[1 byte]   version: 0x01
[4 bytes]  u32 — number of global-init instructions
[N]        global-init instructions
[4 bytes]  u32 — number of functions
  [2 bytes]  u16 — name length
  [N bytes]  UTF-8 name
  [4 bytes]  u32 — number of params
    [4 bytes]  u32 key
    [1 byte]   FasmType tag
    [1 byte]   required flag
    [2 bytes]  u16 name length
    [N bytes]  UTF-8 name
  [4 bytes]  u32 — number of instructions
  [N]        variable-width instructions
```

Each instruction: `[1 byte opcode][1 byte operand count][operands…]`

---

## Fault Codes

| Code | Name | Cause |
|---|---|---|
| `0x01` | `NullDerefFault` | Dereferencing a null reference |
| `0x02` | `IndexOutOfBoundsFault` | VEC/Stack/Queue index out of range |
| `0x03` | `FieldNotFoundFault` | STRUCT key does not exist |
| `0x04` | `DivisionByZeroFault` | DIV or MOD with zero divisor |
| `0x05` | `StackOverflowFault` | Call depth exceeded 512 frames |
| `0x06` | `UnwrapFault` | UNWRAP on wrong variant |
| `0x07` | `WriteAccessViolation` | Write through an immutable reference |
| `0x08` | `TypeMismatch` | Instruction applied to wrong value type |
| `0x09` | `UndeclaredSlot` | Reading an uninitialised slot |
| `0x0A` | `BadSyscall` | Syscall ID not registered |

---

## Crate Reference

| Crate | Purpose |
|---|---|
| [`fasm-bytecode`](crates/fasm-bytecode/) | Opcodes, type tags, `Instruction`/`Operand` model, binary encode/decode |
| [`fasm-compiler`](crates/fasm-compiler/) | Lexer, parser, AST, static validator, two-pass bytecode emitter |
| [`fasm-vm`](crates/fasm-vm/) | `Value` enum, `Frame`, `GlobalRegister`, `Executor`, fault codes |
| [`fasm-sandbox`](crates/fasm-sandbox/) | `Sandbox` isolation wrapper, `ClockController`, IPC sidecar integration |
| [`fasm-cli`](crates/fasm-cli/) | `fasm` binary — `compile` / `run` / `exec` / `check` / `bench` |

---

## Clock Throttling

Sandboxes support an optional instruction-rate limit for controlled execution:

```powershell
.\target\release\fasm.exe run examples\fibonacci.fasm --clock-hz 1000
```

`--clock-hz 0` (default) means unlimited throughput.

---

## Full Language Specification

See **[FASM.md](FASM.md)** for the complete reference: all types, directives, instructions, calling conventions, async model, error handling semantics, and the bytecode encoding format.

---

## License

MIT
