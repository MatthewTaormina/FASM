# FASM Bytecode Format (`.fasmc`) Specification

FASM compiles down to a deterministic, platform-independent bytecode formatted binary file (`.fasmc`). Bytecode binaries map directly into the FASM executing virtual machine sequentially, designed recursively around functions, structs, and variables.

This specification details the canonical encoding representation of FASM's 0x01 bytecode specification structure.

## File Hierarchy and Magic Bounds

The byte stream of a `.fasmc` file encodes standard structures hierarchically. All numbers in multi-byte types map using **Little-Endian (LE)**.

### Header
Every assembled FASM bytecode file begins strictly with the four magic bytes (`FSMC`) mapping out the compatibility prefix, followed by a target architecture bytecode version tag.

```
[0x46 0x53 0x4D 0x43]   magic bytes: b"FSMC"
[1 byte]                version flag: 0x01
```

### Global Initialization Instructions
Globals map to zero or more instructions setting constant types immediately executed when loading the VM runtime.
```
[4 bytes] u32: Total count of globally initializing instructions
<loop instruction count>
    [Encoded Instruction Block]
</loop>
```

### Function Declarations
Following the global instructions, the function manifest kicks in. It captures zero or more function blocks, the first typically being `"Main"`.
```
[4 bytes] u32: Total internal function definitions
<loop function count>
    [2 bytes] u16: Length (N) of the function name
    [N bytes]: Function Name encoded strictly in UTF-8
    
    [4 bytes] u32: Count of the function parameters
    <loop parameter count>
        [4 bytes] u32: Associated struct parameter lookup Key
        [1 byte]  u8:  FasmType tag describing expected type
        [1 byte]  u8:  Required parameter flag (1 = required, 0 = optional)
        [2 bytes] u16: Length (P) of the local tracking param name
        [P bytes]: Local param identifying name mapped in UTF-8
    </loop>
    
    [4 bytes] u32: Internal total instructions count for this function block
    <loop internal instructions count>
        [Encoded Instruction Block]
    </loop>
</loop>
```

---

## Instruction Encoding Layout

An encoded instruction comprises a fixed width opcode, the total numeric lengths of expected operators, and a consecutive payload of those sequential operators themselves mapping to slot variables, immediates, jump labels, and primitive variables.

```
[1 byte] u8: Opcode
[1 byte] u8: Operator count
<loop operator count>
    [1 byte]  u8: Operator Tag (declaring what payload is provided)
    [N bytes]: Dynamic Payload Content
</loop>
```

---

## Opcode Table

Opcodes instruct the virtual machine action loop evaluating execution frames natively.

| Opcode | Hex | Description |
|-----------|---|-------------|
| `Nop` | 0x00 | No operation |
| `Reserve` | 0x01 | Reserve local variable index memory |
| `Release` | 0x02 | Clear local variable index memory |
| `Mov` | 0x03 | Primitive variable duplicate moving |
| `Store` | 0x04 | Write value into memory pointers |
| `Addr` | 0x05 | Cast object reference pointer |
| `Add` | 0x10 | Append Math |
| `Sub` | 0x11 | Subtract Math |
| `Mul` | 0x12 | Multiply Math |
| `Div` | 0x13 | Divide Math |
| `Mod` | 0x14 | Modulus Math |
| `Neg` | 0x15 | Sub-zero inversion math |
| `Eq` | 0x20 | Equality |
| `Neq` | 0x21 | Non-Equality |
| `Lt` | 0x22 | Less than constraint |
| `Lte` | 0x23 | Less or Equal than constraint |
| `Gt` | 0x24 | Greater than constraint |
| `Gte` | 0x25 | Greater or Equal strictness |
| `And` | 0x30 | Binary AND |
| `Or` | 0x31 | Binary OR |
| `Xor` | 0x32 | Binary XOR |
| `Not` | 0x33 | Binary NOT inversion |
| `Shl` | 0x34 | Bitwise structural shift left |
| `Shr` | 0x35 | Bitwise shift right |
| `Jmp` | 0x40 | Non-conditional execution hop |
| `Jz` | 0x41 | Hop strictly against Zero evaluations |
| `Jnz` | 0x42 | Hop strictly barring zero evaluations |
| `Call` | 0x43 | Synchronous function push |
| `AsyncCall` | 0x44 | Threaded asynchronous pushing loop |
| `Ret` | 0x45 | Terminate function pushing back return values |
| `Await` | 0x46 | Execution blocker |
| `TailCall`| 0x47 | Zero-growth caller replacement branch |
| `TmpBlock`| 0x48 | Pushes scoped TMP_BLOCK memory |
| `EndTmp`  | 0x49 | Unwinds scoped local TMP stack memory |
| `Syscall` | 0x50 | External System execution hook |
| `AsyncSyscall` | 0x51 | Threaded out-of-bounds execution hook |
| `Push` | 0x60 | Push Stack variable tracking |
| `Pop` | 0x61 | Pop Stack payload out of top elements |
| `Enqueue` | 0x62 | Queue push mechanics |
| `Dequeue` | 0x63 | Queue pop resolving mechanics |
| `Peek` | 0x64 | Data collection peak |
| `GetIdx` | 0x65 | Positional indexed extraction |
| `SetIdx` | 0x66 | Positional mutational override |
| `GetField` | 0x67 | Object structural extraction by associative Key |
| `SetField` | 0x68 | Object mutational override by relative mapping key |
| `HasField` | 0x69 | Struct constraint Boolean evaluations |
| `DelField` | 0x6A | Associative struct variable dropping maps |
| `Len` | 0x6B | Extraction mapping length enumerations |
| `Prepend` | 0x6C | Deque double shifting insertions |
| `Try` | 0x7E | Exception scoping bounds map handler |
| `Catch` | 0x7F | Exception handler skipping tracker bounds |
| `EndTry` | 0x80 | Terminator of scoped handler tracker |
| `Cast` | 0x90 | Polymorphic trait overriding memory mapper |

*Note: Custom memory modifiers mapped between 0xA0 and 0xFF map advanced FASM dynamic collections (VEC_FILTER_GT, SPARSE_GET, RESULT mappings, etc).*

---

## Operand Component Tags

Operand types are prefixed in arguments by their underlying structural bounds telling the decoder how wide their variable payloads exist globally over reading lengths.

### Referencing Variables
| Tag | Identifier | Payload Format | Description |
|---|---|---|---|
| 0x00 | `TAG_LOCAL` | `[1 byte] u8` | Read/write from calling frame's current local position |
| 0x01 | `TAG_GLOBAL` | `[2 bytes] u16` | Pointer onto the app's persistent generic memory slot |
| 0x02 | `TAG_DEREF_L` | `[1 byte] u8` | Follow pointing bindings resolving local addresses |
| 0x03 | `TAG_DEREF_G` | `[2 bytes] u16` | Traces dereferenced Global execution registers |
| 0x04 | `TAG_TMP` | `[1 byte] u8` | Access bounded temporary t0-t15 variables directly |
| 0x05 | `TAG_DEREF_TMP` | `[1 byte] u8` | Tracks bindings from temporary t0-t15 memory directly |
| 0x06 | `TAG_BUILTIN` | `[1 byte] u8 tag` | Tracks builtin contextual elements dynamically mapped *(0x00=$args, 0x01=$ret, 0x02=$fault_idx, 0x03=$fault_code)* |

### Immediate Primitives
| Tag | Variable Encoding Formats | Width Mapping Constants |
|---|---|---|
| 0x10 | Basic `TAG_IMM_BOOL` | `[1 byte]` (0=false, 1=true) |
| 0x11 | Raw `TAG_IMM_I8` | `[1 byte]` Signed payload bindings |
| 0x12 | Expanded `TAG_IMM_I16` | `[2 bytes]` Little-Endian Integer |
| 0x13 | Common `TAG_IMM_I32` | `[4 bytes]` Native Number Constants |
| 0x14 | Giant `TAG_IMM_I64` | `[8 bytes]` Massive Mapping Bounds |
| 0x15 | Unsigned `TAG_IMM_U8` | `[1 byte]` Raw binary limits |
| 0x16 | Short `TAG_IMM_U16` | `[2 bytes]` Limits maps |
| 0x17 | Unbound `TAG_IMM_U32` | `[4 bytes]` Standard unsigned |
| 0x18 | Ultimate `TAG_IMM_U64`| `[8 bytes]` Total storage bits |
| 0x19 | Floating `TAG_IMM_F32` | `[4 bytes]` IEEE 754 Map |
| 0x1A | Double `TAG_IMM_F64` | `[8 bytes]` Full double IEEE maps |
| 0x1B | Void `TAG_IMM_NULL` | *Null* (No additional bytes) |
| 0x1C | Standard `TAG_IMM_STR` | `[2 bytes len] [UTF-8 bytes]` Expanded locally to VEC<UINT8> implicitly |

### Compiling Logic Definitions
| Tag | Metadata Variables | Description / Payload Width Map |
|---|---|---|
| 0x20 | `TAG_FUNC_REF` | Local `[2 bytes] u16` offset of functional reference tracker pointer |
| 0x21 | `TAG_LABEL` | IP jump tracking bounds returning `[4 bytes] u32` resolving jumps |
| 0x22 | `TAG_SYSCALL_ID` | Mappings resolving OS or hook identifiers `[4 bytes] i32` values |
| 0x23 | `TAG_TYPE` | Maps explicitly to memory typings using a 1-byte `FasmType` Tag enum |
| 0x24 | `TAG_KEY` | Provides integer hash lookups mapping parameters by ID `[4 bytes] u32` |
| 0x25 | `TAG_REQUIRED` | Maps required constraints implicitly tracking validation `[1 byte]` boolean |

---

## Internal Runtime Type Values (FasmType)

When reserving data natively or passing constraints, FASM utilizes internal byte tags defining the precise memory architectures strictly managed natively:

| Byte | Variable Identity Maps | Description Requirements Layout |
|---|---|-------------|
| 0x01-0x0B | Primitives | Internal native mathematical and scalar memory variables |
| 0x10 | RefMut | References bounding mutation writes |
| 0x11 | RefImm | Immutably restricted reading access pointers |
| 0x20 | Vec | Linear raw data lists natively scaled |
| 0x21 | Struct | High mapping integer hashed structural forms |
| 0x22 | Stack | LIFO structure definitions |
| 0x23 | Queue | FIFO definitions |
| 0x24 | HeapMin | Min max priority queue mappings |
| 0x25 | HeapMax | Max priority queue implementations natively |
| 0x26 | Sparse | O(1) indexed variable keys HashMap wrappers |
| 0x27 | BTree | O(log n) ordered tree mappings |
| 0x28 | Slice | 0-copy VEC mapping abstractions explicitly bounded |
| 0x29 | Deque | Prepend/Append variable native queue buffers |
| 0x2A | Bitset | Array mapping booleans specifically |
| 0x2B | Bitvec | Packed binary scalar lists |
| 0x30 | Option | Fallback optional binding variants |
| 0x31 | Result | Fallbacks capturing `[val]/fault` structures |
| 0x32 | Future | Delayed asynchronous executions variables mapping |
| 0xFF | Null | Empty object bindings resolving memory variables |
