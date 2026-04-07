# FASM

## Compiler Directives

Directives used by the FASM assembler during the build process. These are processed *before* code generation.

### INCLUDE "filename.fasm"
Imports the contents of an external FASM file at the directive's location.
- **Constraints**: Relative paths only. Built-in protection prevents circular includes.

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
- **Behavior**: If called within a `FUNC`, it creates a local reservation. Re-reserving an occupied index without a `RELEASE` triggers a `DoubleReserveFault`.

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

### FUNC name
Starts a function definition.
- name: The unique identifier for the function.

### ENDF
Marks the end of a function block. All local reservations are released.

### PARAM index, type, name
Defines an input parameter for the function scope.
- **index**: The numeric slot in the parameter frame (0-based).
- **type**: The FASM type of the parameter.
- **name**: A symbolic alias. The compiler resolves this name to the numeric `index` at compile time — they are interchangeable in all instructions.

### LOCAL index, type, name
Allocates local storage within the function's execution frame.
- **index**: The numeric slot in the local frame (0-based, separate namespace from `PARAM`).
- **type**: The FASM type of the local variable.
- **name**: A symbolic alias resolved to the numeric `index` at compile time.

### CALL name, [args...]
Invokes a defined function.
- **name**: The identifier of the function to call.
- **args**: Ordered list of symbolic names or indices passed as arguments to the callee's `PARAM` frame.
- **Return Value**: After `CALL` returns, the result is available via the special symbol `$ret`.

### RET [value]
Returns from the current function to the caller.
- **value**: Optional. The value or symbolic name to return. Omitting `value` is a void return.
- **Behavior**: All `LOCAL` and `PARAM` slots are released. Execution resumes at the instruction after the originating `CALL`.

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

### Collections

### VEC
A growable, contiguous array of elements.
- **Features**: O(1) random access, automatic resizing.
- **Internal**: Implemented as a capacity-doubling buffer.

### STRUCT
A collection of heterogeneous types.
- **Features**: Fields are accessed by constant offsets defined at compile-time.
- **Internal**: Memory-aligned to the largest member.

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

### GET_FIELD struct, offset, target
Reads the field at compile-time constant `offset` from a `STRUCT` and copies it into `target`.
- **offset**: An integer literal or `DEFINE` constant representing the field's position.

### SET_FIELD struct, offset, value
Writes `value` into the field at `offset` within a `STRUCT`.

### LEN collection, target
Stores the current element count of a `VEC`, `STACK`, `QUEUE`, or `HEAP` into `target` (type `UINT32`).

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

## Scoping

FASM utilizes a **Function-First** architecture. This means:
1. **Entry Point**: The virtual machine begins execution at a function designated as `FUNC Main`.
2. **Frames**: Each function call creates a new execution frame. Indices specified in `PARAM` and `LOCAL` are relative to the current frame pointer, ensuring recursion and isolation.
3. **Implicit Release**: All `LOCAL` memory is automatically released when `ENDF` or `RET` is encountered.

## Example: Recursive Fibonacci

Demonstrates `PARAM`, `LOCAL`, `CMP`, `JNZ`, `CALL`, `$ret`, and `RET`.

```fasm
FUNC Fibonacci
    // Declare inputs
    PARAM 0, INT32, n

    // Declare locals
    LOCAL 0, BOOL,  is_base
    LOCAL 1, INT32, n_minus_1
    LOCAL 2, INT32, n_minus_2
    LOCAL 3, INT32, res1
    LOCAL 4, INT32, res2
    LOCAL 5, INT32, result

    // Base case: if n <= 1, return n
    LTE n, 1, is_base
    JNZ is_base, BaseCase

    // Recursive step: Fibonacci(n-1)
    SUB n, 1, n_minus_1
    CALL Fibonacci, n_minus_1
    MOV $ret, res1

    // Recursive step: Fibonacci(n-2)
    SUB n, 2, n_minus_2
    CALL Fibonacci, n_minus_2
    MOV $ret, res2

    // Sum results
    ADD res1, res2, result
    RET result

BaseCase:
    RET n
ENDF

FUNC Main
    LOCAL 0, INT32, answer
    CALL Fibonacci, 10
    MOV $ret, answer
    // answer == 55
ENDF
```