# Design Note: FASM Register Tags (Semantic Annotations)

**Status:** Tabled — not yet implemented  
**Date:** 2026-04-08

---

## Concept

**Tags** are semantic annotations attached to a **register** (local or parameter)
at declaration time.  They do not change the underlying type or storage — a
tagged `VEC<UINT8>` is still a `VEC<UINT8>` at the VM level.  The tag travels
with the register and tells the engine, syscalls, and the HTTP boundary how to
**format** or **interpret** the value.

```fasm
; All three are VEC<UINT8> at runtime.  The tag is the difference.
LOCAL 0, VEC, name   #utf8    ; human-readable text
LOCAL 1, VEC, auth   #b64     ; base64 view of raw bytes
LOCAL 2, VEC, digest #hex     ; hex-encoded digest
```

Tags are **per-register**, not per-type.  The same type (`VEC<UINT8>`) can hold
any of these, and the tag at the call/return site determines how the engine
handles it.

---

## Why Tags on Registers, Not Types

- **Type system stays clean** — `VEC<UINT8>` is one type, always.  No
  proliferation of `UTF8Vec`, `B64Vec`, `HexVec` variant types in the bytecode.
- **Composable** — a struct field or vec element can carry its own tag
  independently.
- **Retroactively applicable** — existing bytecode is unaffected; tags default
  to `#none` (current behaviour).
- **Familiar** — similar to Rust attributes or LLVM metadata on values.

---

## Proposed Tags

| Tag | Applies to | Semantic |
|-----|-----------|----------|
| `#utf8` | `VEC<UINT8>` | Valid UTF-8 text.  Validated on write.  JSON → plain string. |
| `#ascii` | `VEC<UINT8>` | ASCII subset.  Rejects bytes > 127 on write. |
| `#b64` | `VEC<UINT8>` | Base64 encoding.  JSON uses `{"$b64":"..."}` sentinel. |
| `#hex` | `VEC<UINT8>` | Hex string (e.g. digests, UUIDs).  JSON as `"deadbeef"`. |
| `#bytes` | `VEC<UINT8>` | Explicitly raw bytes.  Never attempt UTF-8; always `$b64`. |
| `#path` | `VEC<UINT8>` | File-system path.  Sandbox can apply allow-list before syscall. |
| `#mime` | `VEC<UINT8>` | MIME type string.  HTTP layer uses for `Content-Type` negotiation. |

---

## Syntax Options (Open)

Three candidates — to be decided:

```fasm
; Option A — inline hash tag suffix (preferred for readability)
LOCAL 0, VEC, payload #b64

; Option B — angle-bracket qualifier on the type keyword
LOCAL 0, VEC<B64>, payload

; Option C — separate attribute line
@tag b64
LOCAL 0, VEC, payload
```

**Lean towards Option A** — least syntax overhead, consistent with how comments
and annotations typically look in low-level languages.

---

## How Tags Flow

```
Declaration site                 Boundary                   Other side
────────────────                 ────────                   ──────────
LOCAL 0, VEC, x   #utf8   ──►   HTTP response   ──►   "plain string"
LOCAL 0, VEC, x   #b64   ──►   HTTP response   ──►   {"$b64":"..."}
LOCAL 0, VEC, x   #hex   ──►   HTTP response   ──►   "deadbeef"
LOCAL 0, VEC, x   #bytes  ──►   HTTP response   ──►   {"$b64":"..."}

; Tags also guide the input side:
; $args field tagged #b64 → engine base64-decodes the incoming JSON string
; $args field tagged #utf8 → engine validates UTF-8 before inserting
```

---

## Implementation Sketch

### Bytecode

Add an optional `tag: u8` field to each `LOCAL` / `PARAM` declaration in the
bytecode.  `0x00` = no tag (current behaviour, fully backwards-compatible).

```
; Extended instruction encoding for LOCAL:
[ OP_LOCAL ] [ reg_idx: u32 ] [ fasm_type: u8 ] [ tag: u8 ] [ name_len: u16 ] [ name: utf8 ]
;                                                  ^^^^^^^^
;                                                  0x00 = none (current)
;                                                  0x01 = utf8
;                                                  0x02 = ascii
;                                                  0x03 = b64
;                                                  0x04 = hex
;                                                  0x05 = bytes
;                                                  0x06 = path
;                                                  0x07 = mime
```

### VM / Executor

- Tags are stored in `LocalRegister` alongside the type and value.
- `SET` instruction: check tag constraint (e.g. reject non-UTF-8 into `#utf8`).
  Strict vs. advisory mode configurable at sandbox level.
- `RET`: tag travels with the returned `Value` via a thin `Tagged<Value>`
  wrapper — or as metadata on the `ExecResult`.

### HTTP Layer (`http_handler.rs`)

On return:
- Inspect tag on returned value → choose serialisation path.
- `#utf8` / `#ascii` → JSON string (same as today for valid UTF-8 `VEC<UINT8>`).
- `#b64` / `#bytes` → `{"$b64":"..."}` sentinel.
- `#hex` → bare hex string `"deadbeef"`.

On input:
- `$args` field tags come from the function's declared parameters.
- Tag guides decoding of the JSON input before insertion into the register.

---

## Open Questions

1. **Validation strictness** — hard fault or soft warn when a tag constraint is
   violated at runtime?
2. **Tag propagation** — when a `#utf8` value is copied into an untagged
   register, does the tag transfer or is it dropped?
3. **Struct fields** — can individual struct fields carry tags?
   `GET_FIELD $args, 0, name #utf8` or declared on the struct definition?
4. **Version bump** — `FSMC` bytecode version byte must be bumped when tags
   are added; old executors should gracefully skip unknown tags.
5. **Compiler syntax** — finalise Option A/B/C above.

---

## References

- `crates/fasm-bytecode/src/types.rs` — `FasmType` enum; add `Tag` annotation
- `crates/fasm-bytecode/src/program.rs` — `LocalDecl` struct to gain `tag: Tag`
- `crates/fasm-compiler/` — parse tag syntax, emit `tag` byte in LOCAL op
- `crates/fasm-vm/src/executor.rs` — enforce tag constraints on SET / RET
- `crates/fasm-engine/src/http_handler.rs` — use tag in `value_to_json` /
  `json_to_value` to eliminate the current `try-UTF-8` heuristic
