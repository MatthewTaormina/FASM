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

## Adding Your Own Example

1. Create `examples/yourprogram.fasm`
2. Declare a `Main` function with no parameters
3. Run with `.\target\debug\fasm.exe run examples\yourprogram.fasm`

See [FASM.md](../FASM.md) for the full language reference, or [README.md](../README.md) for a quick instruction summary.
