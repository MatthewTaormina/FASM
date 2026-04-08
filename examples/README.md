# FASM Examples

## fibonacci.fasm

Computes the nth Fibonacci number using a recursive function.

Demonstrates:
- `DEFINE` compile-time constants as struct field keys
- `PARAM` declarations with `REQUIRED` flag
- Local slot allocation (`LOCAL`)
- Struct creation and field access (`RESERVE`, `SET_FIELD`, `GET_FIELD`)
- Recursive `CALL` with struct arguments
- Conditional branching (`LTE`, `JNZ`, `LABEL`)
- `SYSCALL 0` (built-in `PRINT`)

### Run

```powershell
# From workspace root:
.\target\debug\fasm.exe run examples\fibonacci.fasm
```

Change the `SET_FIELD args, ARG_N, 19` line to any non-negative integer to compute a different term.

| n  | Fibonacci(n) |
|----|-------------|
| 0  | 0           |
| 1  | 1           |
| 5  | 5           |
| 10 | 55          |
| 15 | 610         |
| 19 | 4181        |
| 20 | 6765        |

> **Note**: This is a naive recursive implementation (exponential time). It is deliberately simple to demonstrate the calling convention, not performance.

---

## calculator.fasm / calculator.c

A side-by-side CLI calculator in **FASM** and **C** showing that FASM can express the same high-level logic as a systems language.

Demonstrates:
- **Control flow** — `WHILE`-style REPL loop with `LABEL` / `JMP` / `JNZ`
- **Scoping** — `FUNC` frames, `LOCAL` slots, `PARAM` declarations
- **User I/O** — `SYSCALL 2` (READ line), `SYSCALL 0/1` (PRINT / PRINT_VEC)
- **String literals** — `"Hello, world!"` compiles to `VEC<UINT8>` at runtime
- **Error handling** — `RESULT<INT32>` wrapper with `IS_OK` / `UNWRAP_ERR` / `UNWRAP_OK`
- **SYSCALL 4** (`PARSE_INT`) — converts a `VEC<UINT8>` of digits to `INT32`, mirrors C's `atoi`

### Run (FASM)

```powershell
.\target\debug\fasm.exe run examples\calculator.fasm
```

### Build & Run (C reference)

```bash
gcc -o calculator examples/calculator.c && ./calculator
```

### Example session

```
=== FASM Demo: CLI Calculator ===
Supported operators: +  -  *  /  %
Type 'q' to quit.

Enter first number: 12
Enter operator (+  -  *  /  %): +
Enter second number: 34
Result: 12 + 34 = 46

Enter first number: 100
Enter operator (+  -  *  /  %): /
Enter second number: 0
Error: division by zero.

Enter first number: q
Goodbye.
```

### Language feature comparison

| C concept         | FASM equivalent                                   |
|-------------------|---------------------------------------------------|
| `while(1) {...}`  | `LABEL loop` … `JMP loop`                        |
| Function calls    | `FUNC` / `CALL` with `STRUCT` args               |
| `printf("...")`   | `SET_FIELD ps, 0, "..."` + `SYSCALL 1, ps`       |
| `fgets` / `scanf` | `SYSCALL 2` (READ) + `SYSCALL 4` (PARSE_INT)     |
| Error codes       | `RESULT<INT32>` + `IS_OK` / `UNWRAP_ERR`         |
| `atoi`            | `SYSCALL 4` (PARSE_INT)                          |

---

## Adding Your Own Example

1. Create `examples/yourprogram.fasm`
2. Declare a `Main` function with no parameters
3. Run with `.\target\debug\fasm.exe run examples\yourprogram.fasm`

See [FASM.md](../FASM.md) for the full language reference, or [README.md](../README.md) for a quick instruction summary.
