# fasm-cli

Command-line interface for the FASM virtual machine.

## Commands

```
fasm compile <file.fasm> [-o output.fasmc]   Compile source to .fasmc bytecode
fasm run     <file.fasm> [--clock-hz N]      Compile and execute in one step
fasm check   <file.fasm>                     Validate source only (no execution)
fasm exec    <file.fasmc> [--clock-hz N]     Execute pre-compiled bytecode
```

## Options

| Flag | Default | Description |
|---|---|---|
| `-o <path>` | `<input>.fasmc` | Output path for `compile` |
| `--clock-hz N` | `0` (unlimited) | Instruction rate limit per sandbox tick |

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Compile error, validation error, or runtime fault |
