# fasm-compiler

FASM source text → bytecode compiler.

## Pipeline

```
Source (.fasm)
  ↓  lexer::tokenize()     — produces Vec<Token>
  ↓  parser::parse()       — produces ProgramAst
  ↓  validator::validate() — static analysis (errors as Err(String))
  ↓  emitter::emit()       — produces Program (fasm-bytecode)
```

Entry point: `compile_source(&str) -> Result<Program, String>`

## Two-Pass Emission

The emitter runs two passes over each function body:

1. **Pass 1** (`collect_locals_and_labels`): records local slot indices and label instruction positions (resolving forward jumps).
2. **Pass 2** (`emit_statements`): generates instructions using the positions from pass 1.

## DEFINE Resolution

All `DEFINE name, value` directives produce a constant map that is substituted everywhere `name` is used as an operand — in field keys, syscall IDs, integer literals, etc.
