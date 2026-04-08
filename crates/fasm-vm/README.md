# fasm-vm

The FASM runtime executor.

## Key Types

- **`Value`** — runtime value enum: scalars (`Bool`, `Int8`…`Float64`), collections (`Vec`, `Struct`, `Stack`, `Queue`, `HeapMin`, `HeapMax`), references (`RefMut`, `RefImm`), wrappers (`Option`, `Result`, `Future`)
- **`Fault`** — error codes emitted during execution; caught by `TRY`/`CATCH` blocks
- **`Frame`** — per-function local slot storage (`u8`-indexed `HashMap`)
- **`GlobalRegister`** — cross-function global slot storage (`u32`-indexed `HashMap`)
- **`Executor`** — the main dispatch loop; holds call stack + globals + syscall table

## Syscall Table

Built-in IDs:

| ID | Name | effect |
|---|---|---|
| 0 | PRINT | prints value to stdout |
| 1 | PRINT_VEC | prints `VEC<UINT8>` without newline |
| 2 | READ | reads a line from stdin |
| 3 | EXIT | terminates the process |

Additional handlers can be registered with `Executor::mount_syscall(id, fn)`.

## TRY / CATCH

When a `TRY` instruction is executed, snapshots of `Frame` and `GlobalRegister` are saved in a `TryGuard`. On any fault, memory is rolled back to the snapshot and execution jumps to the `CATCH` label. `ENDTRY` clears the guard on the happy path.
