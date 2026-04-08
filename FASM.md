# FASM

## Compiler Directives

Directives used by the FASM assembler during the build process. These are processed *before* code generation.

### INCLUDE "filename.fasm"
Imports the contents of an external FASM file at the directive's location.
- **Constraints**: Relative paths only. Built-in protection prevents circular includes.

### IMPORT "filename.fasml" AS alias
Loads a pre-compiled shared library.
- **Behavior**: Makes functions from the library available via `alias.FunctionName`.

### DEFINE label, value
Creates a compile-time constant. 
- **Usage**: Use the `label` in place of a literal `value` throughout the code.

### IFDEF label / IFNDEF label
Conditional compilation blocks.

### ELSE / ENDIF
Used with `IFDEF` / `IFNDEF` to branch compiler logic.

### MACRO name, [args...]
Defines a reusable code block.
- **Behavior**: Direct textual substitution. Each expansion creates a unique local namespace for inner labels.

### ENDM
Marks the end of a `MACRO` definition.

### ASSERT condition, "message"
Checks a condition at compile time.
- **Behavior**: If the condition fails, the assembler exits with an error and the provided message.

## Directives

### RESERVE index, type, value
Allocates a memory slot at the specified `index`.
- **index**: The destination slot (0-255 for local frames, unlimited for global).
- **type**: A FASM primitive or collection type.
- **value**: Initial value. If `NULL`, memory is zeroed.
- **Behavior**: If called within a `FUNC`, it creates a local reservation. If called in the **global register** (outside any `FUNC`), it persists for the lifetime of the program.
- **Constraint**: Re-reserving an occupied index without a `RELEASE` triggers a `DoubleReserveFault`.

## Global Register

The global register is a flat memory space **outside all function frames**, populated by `RESERVE` statements declared at the top level (outside any `FUNC` block). Global slots:
- Are allocated once at program load before `FUNC Main` executes.
- Persist for the full lifetime of the VM process.
- Are accessible from **any** function by their global index.
- Use an **unlimited** index space (not capped at 255 like local frames).
- Must be manually `RELEASE`d if cleanup is required; they are never implicitly freed.

Global indices and local frame indices are **separate namespaces**. A global slot at index `5` does not conflict with a local slot at index `5` inside a function.

### RELEASE index
Frees the memory associated with the `index`.
- **index**: The slot to deallocate.
- **Behavior**: Local slots are implicitly released on `RET`. Manual release is required for global or heap-allocated structures.

### COPY src, target
Duplicates the data from `src` into `target`.
- **src/target**: Memory indices.
- **Behavior**: Performs a bitwise copy for scalars. For collections, this is a **shallow copy** (reference transfer).

### MOV src, dst
Transfers data from `src` to `dst`.
- **src/dst**: Memory indices.
- **Behavior**: Unlike `COPY`, this is a move operation. The `src` index is invalidated (set to `NULL`) after the transfer. Highly optimized for large collections.

### STORE value, index
Writes a literal value into a memory slot.
- **value**: An immediate value (e.g., `10`, `3.14`).
- **index**: The destination slot.
- **Constraint**: The `value` must be compatible with the type reserved at the `index`.

## Arithmetic

### ADD src1, src2, target
Adds two values and stores the result in the target index.

### SUB src1, src2, target
Subtracts `src2` from `src1` and stores the result in the target index.

### MUL src1, src2, target
Multiplies two values and stores the result in the target index.

### DIV src1, src2, target
Divides `src1` by `src2` and stores the result in the target index.
(Decimal result if float types, integer division if integer types).

### MOD src1, src2, target
Calculates the remainder of `src1 / src2` and stores it in the target index.

## Functions

All functions and syscalls use a **unified calling convention**: arguments are always passed as a single `STRUCT` argument register. This makes calls self-describing and argument order irrelevant — all values are accessed by key.

### FUNC name
Starts a function definition.
- **name**: The unique identifier for the function.

### ENDF
Marks the end of a function block. Implicitly performs a void `RET` if no explicit `RET` is encountered.

### PARAM key, type, name, REQUIRED | OPTIONAL
Declares an expected field on the incoming argument `STRUCT`.
- **key**: A `UINT32` key matching a field in the passed `STRUCT`.
- **type**: The expected FASM type of that field.
- **name**: A symbolic alias, resolved at compile time. Can be used in place of `key` in all instructions.
- **REQUIRED**: The static validator enforces that any `CALL` to this function provides this key in its argument struct.
- **OPTIONAL**: The field may be absent. Access via `HAS_FIELD` before `GET_FIELD` or a `TRY`/`CATCH` block.
- **Behavior**: `PARAM` declarations do not allocate memory — they describe and validate the incoming struct's shape.
- **Accessing values**: Use `GET_FIELD $args, key, target` where `$args` is the reserved symbol for the incoming argument struct.

### LOCAL index, type, name
Allocates local storage within the function's execution frame.
- **index**: The numeric slot in the local frame (0-based).
- **type**: The FASM type of the local variable.
- **name**: A symbolic alias resolved to the numeric `index` at compile time.

### CALL name, struct
Invokes a defined function, passing a `STRUCT` as the argument register.
- **name**: The identifier of the function to call.
- **struct**: A `STRUCT` slot whose keys satisfy the callee's `REQUIRED` `PARAM` declarations.
- **Validation**: The static validator checks that all `REQUIRED` params are present as keys in the provided struct.
- **Return Value**: After `CALL` returns, the result is available via the special symbol `$ret`.

### ASYNC CALL name, struct
Invokes a function asynchronously.
- **Behavior**: Spawns a new execution context within the current sandbox. Immediately stores a `FUTURE` handle in `$ret`.
- **Usage**: Used to run multiple operations concurrently within a sandbox.

### AWAIT future, target
Suspends the calling execution context until a `FUTURE` resolves.
- **future**: A slot containing a `FUTURE` handle.
- **target**: The slot to store the resolved value into.
- **Behavior**: Yields execution control cooperatively.

### RET [value]
Returns from the current function to the caller.
- **value**: Optional. The value or symbolic name to return. Omitting `value` is a void return.
- **Behavior**: All `LOCAL` memory is released. Execution resumes at the instruction after the originating `CALL` (or marks the `FUTURE` as resolved if called via `ASYNC CALL`).

## Syscalls

Syscalls are the interface between FASM programs and the VM host environment (I/O, OS services, etc.).

### SYSCALL id, struct
Invokes a host-defined system call.
- **id**: An `INT32` literal or slot identifying the syscall. Negative IDs are reserved for the VM host.
- **struct**: A `STRUCT` slot containing the input arguments for the syscall.
- **Return Value**: Available via `$ret` after the call, if the syscall produces one.
- **Behavior**: The host may also write response fields back into the passed `struct`.

### ASYNC SYSCALL id, struct
Invokes a system call asynchronously.
- **Behavior**: Immediately returns a `FUTURE` handle in `$ret` without blocking the sandbox. Use `AWAIT` to get the result.

### Standard Syscall IDs

| ID | Name | Description |
| :--- | :--- | :--- |
| `0` | `PRINT` | Writes a value to standard output. Struct key `0` = value to print. |
| `1` | `PRINT_VEC` | Writes a `VEC` of `UINT8`/`UINT32` as a character sequence. Struct key `0` = vec. |
| `2` | `READ` | Reads a line from standard input. Returns result via `$ret` as a `VEC` of `UINT8`. |
| `3` | `EXIT` | Halts the VM. Struct key `0` = exit code (`INT32`). |

## Control Flow

### LABEL name
Defines a jump target within a function. Labels are **local to the enclosing `FUNC`** — no cross-function jumps are permitted.

### JMP label
Unconditionally jumps to a local label.

### JZ condition, label
Jumps to the label if `condition` evaluates to `0` (false).
- **condition**: A symbolic name or index holding a `BOOL`, integer, or result of a `CMP` instruction.

### JNZ condition, label
Jumps to the label if `condition` evaluates to non-zero (true).

## Comparison

### CMP src1, src2, target
Compares two values and stores a `BOOL` result (`1`=true, `0`=false) in `target`.
- **src1 / src2**: Symbolic names or indices of the same type.
- **target**: Must be a slot of type `BOOL`.
- **Note**: Types must match; comparing `INT32` to `FLOAT32` is a compile-time error unless an explicit `CAST` is performed first.

### EQ src1, src2, target
Stores `1` if `src1 == src2`, else `0`.

### NEQ src1, src2, target
Stores `1` if `src1 != src2`, else `0`.

### LT src1, src2, target
Stores `1` if `src1 < src2`, else `0`.

### LTE src1, src2, target
Stores `1` if `src1 <= src2`, else `0`.

### GT src1, src2, target
Stores `1` if `src1 > src2`, else `0`.

### GTE src1, src2, target
Stores `1` if `src1 >= src2`, else `0`.

## Bitwise Operations

All bitwise instructions operate exclusively on integer types (`INT8`–`UINT32`).

### AND src1, src2, target
Bitwise AND.

### OR src1, src2, target
Bitwise OR.

### XOR src1, src2, target
Bitwise XOR.

### NOT src, target
Bitwise NOT (one's complement).

### SHL src, shift, target
Shifts `src` left by `shift` bits. Vacated bits are zero-filled.

### SHR src, shift, target
Shifts `src` right by `shift` bits. For signed integers, this is an arithmetic shift (sign-extended). For unsigned, zero-filled.

## Types

> **No String Type**: FASM has no native string type. Text must be represented as a `VEC` of integer values (e.g., `UINT8` for ASCII, `UINT32` for Unicode code points). The encoding convention is left to the programmer.

### Collections

### VEC
A growable, contiguous array of elements.
- **Features**: O(1) random access, automatic resizing.
- **Internal**: Implemented as a capacity-doubling buffer.

### STRUCT
A dynamic map of `UINT32` keys to typed values.
- **Features**: Fields are created and accessed by integer key at runtime. No fixed layout or schema required.
- **Internal**: Implemented as a hash map keyed on `UINT32`. Field types are determined by the value assigned.
- **Nesting**: Field values can be any FASM type, including `REF_MUT` or `REF_IMM` pointing to other collections, enabling arbitrarily deep nesting.
- **Convention**: Use `DEFINE` constants to give keys symbolic names (e.g., `DEFINE FIELD_X, 0`).

### STACK
A Last-In-First-Out (LIFO) data structure.
- **Constraints**: Fixed size upon reservation.
- **Instructions**: Requires `PUSH` and `POP` for interaction.

### HEAP_MIN / HEAP_MAX
A binary heap for priority-based storage.
- **Behavior**: Automatically maintains the smallest (`MIN`) or largest (`MAX`) element at the root (index 0).
- **Complexity**: O(log n) for insertions and deletions.

### QUEUE
A First-In-First-Out (FIFO) buffer.
- **Implementation**: Circular buffer to prevent fragmentation.
- **Instructions**: Requires `ENQUEUE` and `DEQUEUE`.

### Wrapper Types

### OPTION
A type representing an optional value.
- **Instructions**: Interacted with via `SOME`, `IS_SOME`, and `UNWRAP`.

### RESULT
A type representing success or a fault code.
- **Instructions**: Interacted with via `OK`, `ERR`, `IS_OK`, `UNWRAP_OK`, and `UNWRAP_ERR`.

### FUTURE
A type representing an asynchronous operation handle.
- **Instructions**: Produced by `ASYNC CALL`/`ASYNC SYSCALL`, consumed by `AWAIT`.

## Collection Instructions

### PUSH collection, value
Pushes a value onto a `STACK` or appends to a `VEC`.

### POP collection, target
Pops the top value from a `STACK` into the target index.

### ENQUEUE queue, value
Adds a value to the back of a `QUEUE`.

### DEQUEUE queue, target
Removes the front value from a `QUEUE` and stores it in the target index.

### PEEK collection, target
Copies the top/front value of a `STACK` or `QUEUE` without removing it.

### GET_IDX vec, index, target
Reads the element at position `index` from a `VEC` or `HEAP_MIN`/`HEAP_MAX` and copies it into `target`.
- **Constraint**: Triggers an `IndexOutOfBoundsFault` if `index >= len(vec)`.

### SET_IDX vec, index, value
Writes `value` into the element at position `index` of a `VEC`.
- **Constraint**: Triggers an `IndexOutOfBoundsFault` if `index >= len(vec)`. Does **not** auto-resize.

### GET_FIELD struct, key, target
Reads the value at `key` from a `STRUCT` and copies it into `target`.
- **key**: A `UINT32` literal, symbolic name, or `DEFINE` constant.
- **Constraint**: Triggers a `FieldNotFoundFault` if the key does not exist.

### SET_FIELD struct, key, value
Writes `value` into the slot at `key` in a `STRUCT`. If the key does not exist, it is created.
- **key**: A `UINT32` literal, symbolic name, or `DEFINE` constant.
- **Behavior**: The field's runtime type is inferred from the assigned `value`.

### HAS_FIELD struct, key, target
Checks whether a `key` exists in a `STRUCT`.
- **key**: A `UINT32` literal, symbolic name, or `DEFINE` constant.
- **target**: Must be a `BOOL` slot. Stores `TRUE` if the key exists, `FALSE` otherwise.

### DEL_FIELD struct, key
Removes the entry at `key` from a `STRUCT`.
- **key**: A `UINT32` literal, symbolic name, or `DEFINE` constant.
- **Constraint**: Silent no-op if the key does not exist.

### LEN collection, target
Stores the current element count of a `VEC`, `STACK`, `QUEUE`, or `HEAP` into `target` (type `UINT32`).

## Wrapper Instructions

### SOME option, value
Wraps a `value` in an `OPTION` slot.
- **option**: A slot of type `OPTION`.

### IS_SOME option, target
Stores `TRUE` in `target` (a `BOOL` slot) if the `OPTION` contains a value, `FALSE` if it is `NULL`.

### UNWRAP option, target
Extracts the inner value of an `OPTION` into `target`.
- **Constraint**: Triggers an `UnwrapFault` if the `OPTION` is `NULL`.

### OK result, value
Wraps a successful `value` in a `RESULT` slot.

### ERR result, fault_code
Wraps a `UINT32` `fault_code` in a `RESULT` slot.

### IS_OK result, target
Stores `TRUE` in `target` if the `RESULT` is `OK`.

### UNWRAP_OK result, target
Extracts the successful value into `target`. Triggers an `UnwrapFault` if it is an `ERR`.

### UNWRAP_ERR result, target
Extracts the fault code into `target`. Triggers an `UnwrapFault` if it is an `OK`.

## Reference Handling

FASM uses a symbolic approach to memory references.

### ADDR src, target
Takes the reference of the memory slot at `src` and stores it into `target`.
- **target**: Must be of type `REF_MUT` or `REF_IMM`.

### Dereference Symbol: &
Appending `&` before a reference slot in any instruction tells the VM to operate on the **pointed-to value** rather than the reference itself.

**Example Usage**:
```fasm
// Standard operations
ADDR my_local, REF_A      // Get reference to my_local
MOV &REF_A, INT_A         // DEREF: Load value from path in REF_A into INT_A
STORE 42, &REF_A          // DEREF: Write 42 directly to relative target of REF_A

// Memory Safety
ADDR my_const, REF_B      // If REF_B is REF_IMM
STORE 50, &REF_B          // ERROR: WriteAccessViolation (IMMUTABLE)
```

### Scalar Primitives

| Type | Bit-Width | Signed | Range |
| :--- | :--- | :--- | :--- |
| **BOOL** | 1-bit | No | `0` (false) or `1` (true) |
| **INT8** | 8-bit | Yes | -128 to 127 |
| **INT16** | 16-bit | Yes | -32,768 to 32,767 |
| **INT32** | 32-bit | Yes | -2,147,483,648 to 2,147,483,647 |
| **INT64** | 64-bit | Yes | -9,223,372,036,854,775,808 to 9,223,372,036,854,775,807 |
| **UINT8** | 8-bit | No | 0 to 255 |
| **UINT16** | 16-bit | No | 0 to 65,535 |
| **UINT32** | 32-bit | No | 0 to 4,294,967,295 |
| **UINT64** | 64-bit | No | 0 to 18,446,744,073,709,551,615 |
| **FLOAT32** | 32-bit | Yes | IEEE 754 Single Precision |
| **FLOAT64** | 64-bit | Yes | IEEE 754 Double Precision |

**Special Values**:
- `NULL` — A zero-value sentinel representing an uninitialized or explicitly empty slot. Valid for all reference and collection types. Using a `NULL` reference with `&` triggers a `NullDerefFault`.
- `TRUE` / `FALSE` — Compile-time aliases for `BOOL` literals `1` and `0`.

### Reference Types

Used for memory aliasing and nesting collections within each other (e.g., a `VEC` of `STRUCT`s).

| Type | Access | Description |
| :--- | :--- | :--- |
| **REF_MUT** | Read/Write | A mutable reference. Supports `&` dereference as a read or write target. |
| **REF_IMM** | Read-Only | An immutable reference. Dereferencing as a write target triggers a `WriteAccessViolation`. |

## Type Conversion

### CAST src, type, target
Converts the value in `src` to `type` and stores the result in `target`.
- **Widening** (e.g., `INT8` → `INT32`): Always safe, zero or sign extended.
- **Narrowing** (e.g., `INT32` → `INT8`): Truncates high bits. No implicit narrowing — must be explicit.
- **Float ↔ Int**: Truncates the decimal on float-to-int conversion.
- **Constraint**: `BOOL` may only be cast to/from integer types.

## Static Validation

Before any instruction is executed, the VM performs a **full pre-run validation pass** over the entire program. Execution does not begin if any error is found. This guarantees that no invalid instruction can occur at runtime.

### Checks Performed

| Check | Error Raised |
| :--- | :--- |
| Referencing an undeclared slot or label | `UndeclaredReferenceError` |
| Type mismatch between instruction and slot type | `TypeMismatchError` |
| Using a `REF_IMM` slot as a write target | `ImmutableWriteError` |
| `CALL` argument count does not match target `PARAM` count | `ArgCountMismatchError` |
| `CALL` argument types do not match target `PARAM` types | `ArgTypeMismatchError` |
| `FUNC` with no `RET` or `ENDF` | `MissingReturnError` |
| `ENDF` or `RET` reached outside a `FUNC` | `ScopeError` |
| Duplicate `FUNC` name | `DuplicateFunctionError` |
| Duplicate `LABEL` name within the same `FUNC` | `DuplicateLabelError` |
| `JMP`/`JZ`/`JNZ` targeting a label in a different `FUNC` | `CrossFunctionJumpError` |
| `DIV` or `MOD` with a literal `0` as the divisor | `StaticDivisionByZeroError` |
| `CAST` between incompatible types (e.g., `BOOL` → `FLOAT32`) | `InvalidCastError` |
| Index out of declared local frame bounds (0-255) | `FrameIndexOverflowError` |

### Runtime-Only Faults
Some conditions cannot be caught statically and are raised at runtime. These can be caught by a `TRY/CATCH` block.

| Fault | Code | Description |
| :--- | :--- | :--- |
| `NullDerefFault` | `0x01` | Dereferencing a `NULL` reference with `&`. |
| `IndexOutOfBoundsFault` | `0x02` | `GET_IDX`/`SET_IDX` on a `VEC` beyond its current length. |
| `FieldNotFoundFault` | `0x03` | `GET_FIELD` on a `STRUCT` key that was never set. |
| `DivisionByZeroFault` | `0x04` | `DIV` or `MOD` where the divisor evaluates to zero at runtime. |
| `StackOverflowFault` | `0x05` | Recursive `CALL` depth exceeded the VM stack limit. |

## Error Handling

FASM provides structured error handling through `TRY`/`CATCH`/`ENDTRY` blocks. When a runtime fault occurs inside a `TRY` block, the VM:
1. **Rolls back** all memory changes made to `LOCAL`, `PARAM`, and global slots during the `TRY` block to their state at `TRY` entry.
2. **Transfers control** to the `CATCH` block.
3. **Exposes** two read-only symbols inside the `CATCH` block:
   - `$fault_index` — the zero-based instruction index within the `TRY` block where the fault occurred (`UINT32`).
   - `$fault_code` — the numeric fault code (`UINT32`, see table above).

### TRY
Begins a fault-guarded block. On entry, the VM snapshots the current state of all memory reachable from the active frame and global register.

### CATCH
Begins the recovery block. Executes only if a runtime fault occurred in the preceding `TRY` block.
- `$fault_index` and `$fault_code` are available as read-only `UINT32` values within this block.
- The `CATCH` block runs with the **rolled-back** memory state — as if the `TRY` block never executed.

### ENDTRY
Closes the `TRY`/`CATCH` structure. If no fault occurred, the `CATCH` block is skipped and execution continues here.

**Example**:
```fasm
FUNC SafeGet
    PARAM 0, VEC,   items
    PARAM 1, UINT32, idx
    LOCAL 0, INT32,  result
    LOCAL 1, UINT32, err

    TRY
        GET_IDX items, idx, result
        RET result
    CATCH
        // items[idx] was out of bounds
        MOV $fault_code, err  // err == 0x02 (IndexOutOfBoundsFault)
        RET -1
    ENDTRY
ENDF
```

**Constraints**:
- `TRY`/`CATCH`/`ENDTRY` are valid only inside a `FUNC` block.
- `TRY` blocks may **not** be nested within each other in the same function.
- `RET` inside a `TRY` or `CATCH` block is valid and exits the function normally.
- Memory rollback covers `LOCAL`, `PARAM`, and **global** slots written during the `TRY` block. Rollback of collection mutations (e.g., `PUSH`, `SET_IDX`) is included.

## Scoping

FASM utilizes a **Function-First** architecture. This means:
1. **Entry Point**: The virtual machine begins execution at `FUNC Main`. `RET` from `Main` (or reaching its `ENDF`) halts the VM.
2. **Frames**: Each `CALL` creates a new execution frame. `PARAM` and `LOCAL` indices are relative to the current frame pointer, ensuring isolation and safe recursion.
3. **Implicit Release**: All `LOCAL` and `PARAM` memory is automatically released when `ENDF` or `RET` is encountered.
4. **Globals Persist**: Global register slots are never implicitly released and outlive all function calls.

## Example: Recursive Fibonacci

Demonstrates the struct-based calling convention, `PARAM`, `LOCAL`, `LTE`, `JNZ`, `CALL`, `$args`, `$ret`, and `SYSCALL`.

```fasm
// DEFINE symbolic keys for the argument struct
DEFINE ARG_N, 0

FUNC Fibonacci
    // Declare expected argument struct fields
    PARAM ARG_N, INT32, n, REQUIRED

    // Declare locals
    LOCAL 0, BOOL,   is_base
    LOCAL 1, INT32,  n_minus_1
    LOCAL 2, INT32,  n_minus_2
    LOCAL 3, INT32,  res1
    LOCAL 4, INT32,  res2
    LOCAL 5, INT32,  result
    LOCAL 6, STRUCT, call_args

    // Read n from the incoming argument struct
    GET_FIELD $args, n, n

    // Base case: if n <= 1, return n
    LTE n, 1, is_base
    JNZ is_base, BaseCase

    // Recursive step: Fibonacci(n-1)
    SUB n, 1, n_minus_1
    RESERVE 6, STRUCT, NULL
    SET_FIELD call_args, ARG_N, n_minus_1
    CALL Fibonacci, call_args
    MOV $ret, res1

    // Recursive step: Fibonacci(n-2)
    SUB n, 2, n_minus_2
    SET_FIELD call_args, ARG_N, n_minus_2
    CALL Fibonacci, call_args
    MOV $ret, res2

    // Sum results
    ADD res1, res2, result
    RET result

BaseCase:
    RET n
ENDF

FUNC Main
    LOCAL 0, INT32,  answer
    LOCAL 1, STRUCT, args
    LOCAL 2, STRUCT, print_args

    // Call Fibonacci(10)
    RESERVE 1, STRUCT, NULL
    SET_FIELD args, ARG_N, 10
    CALL Fibonacci, args
    MOV $ret, answer
    // answer == 55

    // Print the result via syscall
    RESERVE 2, STRUCT, NULL
    SET_FIELD print_args, 0, answer
    SYSCALL 0, print_args
ENDF
```