# FASM — Function-first Assembly

A high-performance, sandboxed virtual machine language designed to be compiled from any language and run anywhere. FASM programs compile to compact `.fasmc` bytecode executed by a Rust-based VM with cooperative async support, memory-safe isolation, and rich collection types.

---

## Quick Start

**Prerequisites**: Rust toolchain (`cargo`)

```powershell
# Build
cargo build --release

# Run a FASM program
.\target\release\fasm.exe run examples\fibonacci.fasm

# Compile to bytecode
.\target\release\fasm.exe compile examples\fibonacci.fasm -o out.fasmc

# Execute pre-compiled bytecode
.\target\release\fasm.exe exec out.fasmc

# Static validation only (no execution)
.\target\release\fasm.exe check examples\fibonacci.fasm
```

### Install globally

```powershell
cargo install --path crates\fasm-cli
# Then use 'fasm' anywhere:
fasm run examples\fibonacci.fasm
```

---

## Workspace Layout

```
FASM/
├── FASM.md                  ← Full language specification
├── Cargo.toml               ← Workspace manifest
├── crates/
│   ├── fasm-bytecode/       ← Instruction model, opcodes, binary encode/decode
│   ├── fasm-vm/             ← Runtime executor, memory, fault handling
│   ├── fasm-compiler/       ← Lexer → Parser → Validator → Emitter
│   ├── fasm-sandbox/        ← Isolated execution context, clock throttling
│   └── fasm-cli/            ← CLI binary (compile / run / check / exec)
├── examples/
│   └── fibonacci.fasm       ← Recursive Fibonacci example
└── tools/
    └── disasm.rs            ← Debug disassembler utility
```

---

## Language Overview

FASM (Function-first Assembly) is a typed, frame-based assembly language with:

- **Typed slots** — every local and global slot has a declared type (`INT32`, `STRUCT`, `VEC`, …)
- **Struct-based calling convention** — all functions and syscalls receive a `STRUCT` argument
- **Rich collection types** — `VEC`, `STRUCT`, `STACK`, `QUEUE`, `HEAP_MIN`, `HEAP_MAX`
- **Wrapper types** — `OPTION`, `RESULT`, `FUTURE`
- **References** — `REF_MUT` / `REF_IMM` and `&` dereference syntax
- **Transactional error handling** — `TRY` / `CATCH` / `ENDTRY` with automatic memory rollback
- **Cooperative async** — `ASYNC CALL` returns a `FUTURE`, `AWAIT` resolves it
- **Global register** — unlimited u32-indexed global slots shared across functions
- **Static validation** — invalid code is rejected before execution

### Hello World

```fasm
FUNC Main
    LOCAL 0, STRUCT, args

    RESERVE 0, STRUCT, NULL
    SET_FIELD args, 0, "Hello, World!"
    SYSCALL 0, args             // PRINT syscall
ENDF
```

### Fibonacci (recursive)

```fasm
DEFINE ARG_N, 0

FUNC Fibonacci
    PARAM ARG_N, INT32, n, REQUIRED

    LOCAL 0, BOOL,   is_base
    LOCAL 1, INT32,  n_val
    LOCAL 2, INT32,  n_minus_1
    LOCAL 3, INT32,  n_minus_2
    LOCAL 4, INT32,  res1
    LOCAL 5, INT32,  res2
    LOCAL 6, INT32,  result
    LOCAL 7, STRUCT, call_args

    GET_FIELD $args, ARG_N, n_val

    LABEL BaseCheck
    LTE n_val, 1, is_base
    JNZ is_base, BaseCase

    SUB n_val, 1, n_minus_1
    RESERVE 7, STRUCT, NULL
    SET_FIELD call_args, ARG_N, n_minus_1
    CALL Fibonacci, call_args
    MOV $ret, res1

    SUB n_val, 2, n_minus_2
    SET_FIELD call_args, ARG_N, n_minus_2
    CALL Fibonacci, call_args
    MOV $ret, res2

    ADD res1, res2, result
    RET result

    LABEL BaseCase
    RET n_val
ENDF

FUNC Main
    LOCAL 0, INT32,  answer
    LOCAL 1, STRUCT, args
    LOCAL 2, STRUCT, print_args

    RESERVE 1, STRUCT, NULL
    SET_FIELD args, ARG_N, 19
    CALL Fibonacci, args
    MOV $ret, answer

    RESERVE 2, STRUCT, NULL
    SET_FIELD print_args, 0, answer
    SYSCALL 0, print_args
ENDF
```

---

## Instruction Set Summary

| Category | Instructions |
|---|---|
| Memory | `RESERVE`, `RELEASE` |
| Data movement | `MOV`, `STORE`, `ADDR` |
| Arithmetic | `ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `NEG` |
| Comparison | `EQ`, `NEQ`, `LT`, `LTE`, `GT`, `GTE` |
| Bitwise | `AND`, `OR`, `XOR`, `NOT`, `SHL`, `SHR` |
| Control flow | `JMP`, `JZ`, `JNZ`, `CALL`, `RET`, `HALT` |
| Async | `ASYNC CALL`, `ASYNC SYSCALL`, `AWAIT` |
| Syscall | `SYSCALL`, `ASYNC SYSCALL` |
| Vec/Stack/Queue | `PUSH`, `POP`, `ENQUEUE`, `DEQUEUE`, `PEEK`, `GET_IDX`, `SET_IDX`, `LEN` |
| Struct | `GET_FIELD`, `SET_FIELD`, `HAS_FIELD`, `DEL_FIELD` |
| Wrappers | `SOME`, `IS_SOME`, `UNWRAP`, `OK`, `ERR`, `IS_OK`, `UNWRAP_OK`, `UNWRAP_ERR` |
| Cast | `CAST` |
| Error handling | `TRY`, `CATCH`, `ENDTRY` |

---

## Built-in Syscalls

| ID | Name | Args (struct key → value) | Returns |
|---|---|---|---|
| `0` | `PRINT` | `0` → any value | `NULL` |
| `1` | `PRINT_VEC` | `0` → `VEC` of `UINT8` (no newline) | `NULL` |
| `2` | `READ` | — | `VEC` of `UINT8` (stdin line) |
| `3` | `EXIT` | `0` → `INT32` exit code | (terminates) |

Custom syscalls can be registered at runtime by the host via `Sandbox::mount_syscall(id, handler)`.

---

## Bytecode Format (`.fasmc`)

```
[4 bytes]  magic: "FSMC"
[1 byte]   version: 0x01
[4 bytes]  u32 — number of global-init instructions
[N]        global-init instructions
[4 bytes]  u32 — number of functions
<per function>
  [2 bytes]  u16 — name length
  [N bytes]  name (UTF-8)
  [4 bytes]  u32 — number of params
  <per param>
    [4 bytes]  u32 key
    [1 byte]   FasmType tag
    [1 byte]   required flag
    [2 bytes]  u16 name length
    [N bytes]  name (UTF-8)
  [4 bytes]  u32 — number of instructions
  [N]        instructions (variable-width)
```

Each instruction:
```
[1 byte]  opcode
[1 byte]  operand count
<per operand>
  [1 byte]  operand kind tag
  [N bytes] operand payload (tag-dependent)
```

---

## Fault Codes

| Code | Name | Cause |
|---|---|---|
| `0x01` | `NullDerefFault` | Dereferencing a null reference |
| `0x02` | `IndexOutOfBoundsFault` | VEC/Stack/Queue index out of range |
| `0x03` | `FieldNotFoundFault` | STRUCT key does not exist |
| `0x04` | `DivisionByZeroFault` | DIV or MOD with zero divisor |
| `0x05` | `StackOverflowFault` | Call depth exceeded 512 frames |
| `0x06` | `UnwrapFault` | UNWRAP / UNWRAP_OK / UNWRAP_ERR on wrong variant |
| `0x07` | `WriteAccessViolation` | Write through an immutable reference |
| `0x08` | `TypeMismatch` | Instruction applied to wrong value type |
| `0x09` | `UndeclaredSlot` | Reading an uninitialised slot |
| `0x0A` | `BadSyscall` | Syscall ID not registered |

---

## Crate Reference

| Crate | Purpose |
|---|---|
| [`fasm-bytecode`](crates/fasm-bytecode/) | Opcodes, type tags, `Instruction`/`Operand` model, binary encode/decode |
| [`fasm-vm`](crates/fasm-vm/) | `Value` enum, `Frame`, `GlobalRegister`, `Executor`, `Fault` codes |
| [`fasm-compiler`](crates/fasm-compiler/) | Lexer, parser, AST, static validator, two-pass bytecode emitter |
| [`fasm-sandbox`](crates/fasm-sandbox/) | `Sandbox` isolation wrapper, `ClockController` throttling |
| [`fasm-cli`](crates/fasm-cli/) | `fasm` binary — compile / run / check / exec commands |

---

## Clock Throttling

Sandboxes support an optional instructions-per-tick limit for controlled execution speed:

```powershell
.\fasm.exe run examples\fibonacci.fasm --clock-hz 1000
```

Setting `--clock-hz 0` (the default) means unlimited speed.

---

## Full Language Specification

See **[FASM.md](FASM.md)** for the complete language reference including all types, directives, instructions, calling conventions, async model, and error handling semantics.

---

## License

MIT
