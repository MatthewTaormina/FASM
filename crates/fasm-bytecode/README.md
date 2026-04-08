# fasm-bytecode

Core instruction model and binary file format for the FASM virtual machine.

## Contents

| Module | Description |
|---|---|
| `opcode` | `Opcode` enum — all ~60 VM opcodes, `u8`-backed with `TryFrom<u8>` |
| `types` | `FasmType` enum — all type tags (primitives, collections, wrappers) |
| `instruction` | `Instruction`, `Operand`, `SlotRef`, `BuiltIn`, `Immediate` |
| `program` | `Program`, `FunctionDef`, `ParamDescriptor` |
| `encode` | `encode_program` / `decode_program` — binary `.fasmc` round-trip |

## Binary Format

```
"FSMC"          4 bytes — magic
0x01            1 byte  — version
<u32>           global init count
<instructions>
<u32>           function count
<functions...>
```

Each instruction: `[opcode u8][operand_count u8][operands...]`

Operands are tagged: `[kind u8][payload...]`
