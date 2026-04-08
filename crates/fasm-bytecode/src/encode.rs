use crate::instruction::{BuiltIn, Immediate, Operand, SlotRef};
use crate::types::FasmType;
/// Binary serialisation / deserialisation for FASM programs.
///
/// File format:
///   [4 bytes] magic: b"FSMC"
///   [1 byte]  version
///   [4 bytes] u32 — number of global-init instructions
///   [N]       encoded global-init instructions
///   [4 bytes] u32 — number of functions
///   <foreach function>
///     [2 bytes] u16 — name length
///     [N bytes] name utf-8
///     [4 bytes] u32 — number of params
///     <foreach param>
///       [4 bytes]  u32  key
///       [1 byte]   u8   FasmType tag
///       [1 byte]   u8   required (1=yes)
///       [2 bytes]  u16  name length
///       [N bytes]  name utf-8
///     [4 bytes] u32 — number of instructions
///     [N]       encoded instructions
///
/// Each instruction:
///   [1 byte] opcode
///   [1 byte] operand count
///   <foreach operand>
///     [1 byte] operand kind tag
///     [N bytes] operand payload
use crate::{FunctionDef, Instruction, Opcode, ParamDescriptor, Program};

pub const MAGIC: &[u8; 4] = b"FSMC";

// ── Operand kind tags ────────────────────────────────────────────────────────
const TAG_LOCAL: u8 = 0x00;
const TAG_GLOBAL: u8 = 0x01;
const TAG_DEREF_L: u8 = 0x02;
const TAG_DEREF_G: u8 = 0x03;
const TAG_TMP: u8 = 0x04;
const TAG_DEREF_TMP: u8 = 0x05;
const TAG_BUILTIN: u8 = 0x06;
const TAG_IMM_BOOL: u8 = 0x10;
const TAG_IMM_I8: u8 = 0x11;
const TAG_IMM_I16: u8 = 0x12;
const TAG_IMM_I32: u8 = 0x13;
const TAG_IMM_I64: u8 = 0x14;
const TAG_IMM_U8: u8 = 0x15;
const TAG_IMM_U16: u8 = 0x16;
const TAG_IMM_U32: u8 = 0x17;
const TAG_IMM_U64: u8 = 0x18;
const TAG_IMM_F32: u8 = 0x19;
const TAG_IMM_F64: u8 = 0x1A;
const TAG_IMM_NULL: u8 = 0x1B;
const TAG_IMM_STR: u8 = 0x1C; // UTF-8 string literal → VEC<UINT8> at runtime
const TAG_FUNC_REF: u8 = 0x20;
const TAG_LABEL: u8 = 0x21;
const TAG_SYSCALL_ID: u8 = 0x22;
const TAG_TYPE: u8 = 0x23;
const TAG_KEY: u8 = 0x24;
const TAG_REQUIRED: u8 = 0x25;

// ── Built-in sub-tags ────────────────────────────────────────────────────────
const BUILTIN_ARGS: u8 = 0x00;
const BUILTIN_RET: u8 = 0x01;
const BUILTIN_FAULT_IDX: u8 = 0x02;
const BUILTIN_FAULT_CODE: u8 = 0x03;

// ─────────────────────────────────────────────────────────────────────────────
// Encode
// ─────────────────────────────────────────────────────────────────────────────

pub fn encode_program(prog: &Program) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(prog.version);

    // global inits
    out.extend_from_slice(&(prog.global_inits.len() as u32).to_le_bytes());
    for instr in &prog.global_inits {
        encode_instruction(instr, &mut out);
    }

    // functions
    out.extend_from_slice(&(prog.functions.len() as u32).to_le_bytes());
    for func in &prog.functions {
        encode_string(&func.name, &mut out);
        out.extend_from_slice(&(func.params.len() as u32).to_le_bytes());
        for p in &func.params {
            out.extend_from_slice(&p.key.to_le_bytes());
            out.push(p.fasm_type as u8);
            out.push(p.required as u8);
            encode_string(&p.name, &mut out);
        }
        out.extend_from_slice(&(func.instructions.len() as u32).to_le_bytes());
        for instr in &func.instructions {
            encode_instruction(instr, &mut out);
        }
    }
    out
}

fn encode_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn encode_instruction(instr: &Instruction, out: &mut Vec<u8>) {
    out.push(instr.opcode as u8);
    out.push(instr.operands.len() as u8);
    for op in &instr.operands {
        encode_operand(op, out);
    }
}

fn encode_operand(op: &Operand, out: &mut Vec<u8>) {
    match op {
        Operand::Slot(s) => match s {
            SlotRef::Local(i) => {
                out.push(TAG_LOCAL);
                out.push(*i);
            }
            SlotRef::Global(i) => {
                out.push(TAG_GLOBAL);
                out.extend_from_slice(&i.to_le_bytes());
            }
            SlotRef::Tmp(i) => {
                out.push(TAG_TMP);
                out.push(*i);
            }
            SlotRef::DerefLocal(i) => {
                out.push(TAG_DEREF_L);
                out.push(*i);
            }
            SlotRef::DerefGlobal(i) => {
                out.push(TAG_DEREF_G);
                out.extend_from_slice(&i.to_le_bytes());
            }
            SlotRef::DerefTmp(i) => {
                out.push(TAG_DEREF_TMP);
                out.push(*i);
            }
            SlotRef::BuiltIn(b) => {
                out.push(TAG_BUILTIN);
                out.push(match b {
                    BuiltIn::Args => BUILTIN_ARGS,
                    BuiltIn::Ret => BUILTIN_RET,
                    BuiltIn::FaultIndex => BUILTIN_FAULT_IDX,
                    BuiltIn::FaultCode => BUILTIN_FAULT_CODE,
                });
            }
        },
        Operand::Imm(imm) => match imm {
            Immediate::Bool(v) => {
                out.push(TAG_IMM_BOOL);
                out.push(*v as u8);
            }
            Immediate::Int8(v) => {
                out.push(TAG_IMM_I8);
                out.push(*v as u8);
            }
            Immediate::Int16(v) => {
                out.push(TAG_IMM_I16);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Int32(v) => {
                out.push(TAG_IMM_I32);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Int64(v) => {
                out.push(TAG_IMM_I64);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Uint8(v) => {
                out.push(TAG_IMM_U8);
                out.push(*v);
            }
            Immediate::Uint16(v) => {
                out.push(TAG_IMM_U16);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Uint32(v) => {
                out.push(TAG_IMM_U32);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Uint64(v) => {
                out.push(TAG_IMM_U64);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Float32(v) => {
                out.push(TAG_IMM_F32);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Float64(v) => {
                out.push(TAG_IMM_F64);
                out.extend_from_slice(&v.to_le_bytes());
            }
            Immediate::Null => out.push(TAG_IMM_NULL),
            Immediate::Str(s) => {
                out.push(TAG_IMM_STR);
                encode_string(s, out);
            }
        },
        Operand::FuncRef(i) => {
            out.push(TAG_FUNC_REF);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Operand::LabelTarget(a) => {
            out.push(TAG_LABEL);
            out.extend_from_slice(&a.to_le_bytes());
        }
        Operand::SyscallId(i) => {
            out.push(TAG_SYSCALL_ID);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Operand::Type(t) => {
            out.push(TAG_TYPE);
            out.push(*t as u8);
        }
        Operand::Key(k) => {
            out.push(TAG_KEY);
            out.extend_from_slice(&k.to_le_bytes());
        }
        Operand::Required(r) => {
            out.push(TAG_REQUIRED);
            out.push(*r as u8);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Decode
// ─────────────────────────────────────────────────────────────────────────────

pub fn decode_program(data: &[u8]) -> Result<Program, String> {
    let mut c = Cursor::new(data);
    let magic = c.read_bytes(4)?;
    if magic != MAGIC {
        return Err("Invalid magic bytes — not a FASMC file".into());
    }
    let version = c.read_u8()?;
    let mut prog = Program::new();
    prog.version = version;

    let nglobals = c.read_u32()?;
    for _ in 0..nglobals {
        prog.global_inits.push(decode_instruction(&mut c)?);
    }

    let nfuncs = c.read_u32()?;
    for _ in 0..nfuncs {
        let name = c.read_string()?;
        let nparams = c.read_u32()?;
        let mut params = Vec::new();
        for _ in 0..nparams {
            let key = c.read_u32()?;
            let t = FasmType::try_from(c.read_u8()?)?;
            let req = c.read_u8()? != 0;
            let pname = c.read_string()?;
            params.push(ParamDescriptor {
                key,
                fasm_type: t,
                name: pname,
                required: req,
            });
        }
        let ninstrs = c.read_u32()?;
        let mut instructions = Vec::new();
        for _ in 0..ninstrs {
            instructions.push(decode_instruction(&mut c)?);
        }
        prog.functions.push(FunctionDef {
            name,
            params,
            instructions,
        });
    }
    Ok(prog)
}

fn decode_instruction(c: &mut Cursor) -> Result<Instruction, String> {
    let opcode = Opcode::try_from(c.read_u8()?)?;
    let nops = c.read_u8()? as usize;
    let mut operands = Vec::with_capacity(nops);
    for _ in 0..nops {
        operands.push(decode_operand(c)?);
    }
    Ok(Instruction { opcode, operands })
}

fn decode_operand(c: &mut Cursor) -> Result<Operand, String> {
    let tag = c.read_u8()?;
    match tag {
        TAG_LOCAL => Ok(Operand::Slot(SlotRef::Local(c.read_u8()?))),
        TAG_GLOBAL => Ok(Operand::Slot(SlotRef::Global(c.read_u16()?))),
        TAG_TMP => Ok(Operand::Slot(SlotRef::Tmp(c.read_u8()?))),
        TAG_DEREF_L => Ok(Operand::Slot(SlotRef::DerefLocal(c.read_u8()?))),
        TAG_DEREF_G => Ok(Operand::Slot(SlotRef::DerefGlobal(c.read_u16()?))),
        TAG_DEREF_TMP => Ok(Operand::Slot(SlotRef::DerefTmp(c.read_u8()?))),
        TAG_BUILTIN => Ok(Operand::Slot(SlotRef::BuiltIn(match c.read_u8()? {
            BUILTIN_ARGS => BuiltIn::Args,
            BUILTIN_RET => BuiltIn::Ret,
            BUILTIN_FAULT_IDX => BuiltIn::FaultIndex,
            BUILTIN_FAULT_CODE => BuiltIn::FaultCode,
            b => return Err(format!("Unknown builtin tag 0x{:02X}", b)),
        }))),
        TAG_IMM_BOOL => Ok(Operand::Imm(Immediate::Bool(c.read_u8()? != 0))),
        TAG_IMM_I8 => Ok(Operand::Imm(Immediate::Int8(c.read_u8()? as i8))),
        TAG_IMM_I16 => Ok(Operand::Imm(Immediate::Int16(c.read_i16()?))),
        TAG_IMM_I32 => Ok(Operand::Imm(Immediate::Int32(c.read_i32()?))),
        TAG_IMM_I64 => Ok(Operand::Imm(Immediate::Int64(c.read_i64()?))),
        TAG_IMM_U8 => Ok(Operand::Imm(Immediate::Uint8(c.read_u8()?))),
        TAG_IMM_U16 => Ok(Operand::Imm(Immediate::Uint16(c.read_u16()?))),
        TAG_IMM_U32 => Ok(Operand::Imm(Immediate::Uint32(c.read_u32()?))),
        TAG_IMM_U64 => Ok(Operand::Imm(Immediate::Uint64(c.read_u64()?))),
        TAG_IMM_F32 => Ok(Operand::Imm(Immediate::Float32(c.read_f32()?))),
        TAG_IMM_F64 => Ok(Operand::Imm(Immediate::Float64(c.read_f64()?))),
        TAG_IMM_NULL => Ok(Operand::Imm(Immediate::Null)),
        TAG_IMM_STR => Ok(Operand::Imm(Immediate::Str(c.read_string()?))),
        TAG_FUNC_REF => Ok(Operand::FuncRef(c.read_u16()?)),
        TAG_LABEL => Ok(Operand::LabelTarget(c.read_u32()?)),
        TAG_SYSCALL_ID => Ok(Operand::SyscallId(c.read_i32()?)),
        TAG_TYPE => Ok(Operand::Type(FasmType::try_from(c.read_u8()?)?)),
        TAG_KEY => Ok(Operand::Key(c.read_u32()?)),
        TAG_REQUIRED => Ok(Operand::Required(c.read_u8()? != 0)),
        _ => Err(format!("Unknown operand tag 0x{:02X}", tag)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cursor helper
// ─────────────────────────────────────────────────────────────────────────────

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.data.len() {
            return Err("Unexpected EOF".into());
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }
    fn read_u16(&mut self) -> Result<u16, String> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn read_u32(&mut self) -> Result<u32, String> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_u64(&mut self) -> Result<u64, String> {
        let b = self.read_bytes(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
    fn read_i16(&mut self) -> Result<i16, String> {
        Ok(self.read_u16()? as i16)
    }
    fn read_i32(&mut self) -> Result<i32, String> {
        Ok(self.read_u32()? as i32)
    }
    fn read_i64(&mut self) -> Result<i64, String> {
        Ok(self.read_u64()? as i64)
    }
    fn read_f32(&mut self) -> Result<f32, String> {
        let b = self.read_bytes(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_f64(&mut self) -> Result<f64, String> {
        let b = self.read_bytes(8)?;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }
    fn read_bytes(&mut self, n: usize) -> Result<&[u8], String> {
        if self.pos + n > self.data.len() {
            return Err("Unexpected EOF".into());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn read_string(&mut self) -> Result<String, String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        instruction::{BuiltIn, Immediate, Operand, SlotRef},
        types::FasmType,
        FunctionDef, Instruction, Opcode, ParamDescriptor, Program,
    };

    fn round_trip(prog: &Program) -> Program {
        let bytes = encode_program(prog);
        decode_program(&bytes).expect("decode must succeed")
    }

    fn simple_program(instrs: Vec<Instruction>) -> Program {
        let mut prog = Program::new();
        prog.functions.push(FunctionDef {
            name: "Main".into(),
            params: vec![],
            instructions: instrs,
        });
        prog
    }

    // ── Magic / version ───────────────────────────────────────────────────────

    #[test]
    fn test_magic_bytes_prefix() {
        let prog = simple_program(vec![Instruction::no_args(Opcode::Halt)]);
        let bytes = encode_program(&prog);
        assert_eq!(&bytes[..4], MAGIC);
        assert_eq!(bytes[4], 0x01, "version must be 0x01");
    }

    #[test]
    fn test_bad_magic_returns_error() {
        let mut bytes = encode_program(&simple_program(vec![]));
        bytes[0] = 0x00; // corrupt magic
        assert!(decode_program(&bytes).is_err());
    }

    #[test]
    fn test_truncated_input_returns_error() {
        assert!(decode_program(&[]).is_err());
        assert!(decode_program(&[b'F', b'S', b'M', b'C']).is_err());
    }

    // ── Empty program round-trip ──────────────────────────────────────────────

    #[test]
    fn test_empty_program_round_trip() {
        let prog = simple_program(vec![]);
        let rt = round_trip(&prog);
        assert_eq!(rt.version, 0x01);
        assert_eq!(rt.functions.len(), 1);
        assert_eq!(rt.functions[0].name, "Main");
        assert!(rt.functions[0].instructions.is_empty());
    }

    // ── Slot operand variants ─────────────────────────────────────────────────

    #[test]
    fn test_slot_local_round_trip() {
        let instr = Instruction::new(Opcode::Mov, vec![Operand::Slot(SlotRef::Local(7))]);
        let rt = round_trip(&simple_program(vec![instr]));
        assert_eq!(
            rt.functions[0].instructions[0].operands[0],
            Operand::Slot(SlotRef::Local(7))
        );
    }

    #[test]
    fn test_slot_global_round_trip() {
        let instr = Instruction::new(Opcode::Mov, vec![Operand::Slot(SlotRef::Global(300))]);
        let rt = round_trip(&simple_program(vec![instr]));
        assert_eq!(
            rt.functions[0].instructions[0].operands[0],
            Operand::Slot(SlotRef::Global(300))
        );
    }

    #[test]
    fn test_slot_tmp_round_trip() {
        let instr = Instruction::new(Opcode::Mov, vec![Operand::Slot(SlotRef::Tmp(3))]);
        let rt = round_trip(&simple_program(vec![instr]));
        assert_eq!(
            rt.functions[0].instructions[0].operands[0],
            Operand::Slot(SlotRef::Tmp(3))
        );
    }

    #[test]
    fn test_deref_slots_round_trip() {
        for op in [
            Operand::Slot(SlotRef::DerefLocal(1)),
            Operand::Slot(SlotRef::DerefGlobal(500)),
            Operand::Slot(SlotRef::DerefTmp(2)),
        ] {
            let instr = Instruction::new(Opcode::Mov, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    #[test]
    fn test_builtin_slots_round_trip() {
        for builtin in [
            BuiltIn::Args,
            BuiltIn::Ret,
            BuiltIn::FaultIndex,
            BuiltIn::FaultCode,
        ] {
            let op = Operand::Slot(SlotRef::BuiltIn(builtin));
            let instr = Instruction::new(Opcode::Mov, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    // ── Immediate variants ────────────────────────────────────────────────────

    #[test]
    fn test_immediate_bool_round_trip() {
        for v in [true, false] {
            let op = Operand::Imm(Immediate::Bool(v));
            let instr = Instruction::new(Opcode::Store, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    #[test]
    fn test_immediate_integers_round_trip() {
        let ops: Vec<Operand> = vec![
            Operand::Imm(Immediate::Int8(-42)),
            Operand::Imm(Immediate::Int16(-1000)),
            Operand::Imm(Immediate::Int32(-100_000)),
            Operand::Imm(Immediate::Int64(-1_000_000_000_000)),
            Operand::Imm(Immediate::Uint8(255)),
            Operand::Imm(Immediate::Uint16(65535)),
            Operand::Imm(Immediate::Uint32(0xDEAD_BEEF)),
            Operand::Imm(Immediate::Uint64(u64::MAX)),
        ];
        for op in ops {
            let instr = Instruction::new(Opcode::Store, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    #[test]
    fn test_immediate_floats_round_trip() {
        let op32 = Operand::Imm(Immediate::Float32(3.14_f32));
        let op64 = Operand::Imm(Immediate::Float64(2.718_281_828_459_045_f64));
        for op in [op32, op64] {
            let instr = Instruction::new(Opcode::Store, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    #[test]
    fn test_immediate_null_round_trip() {
        let op = Operand::Imm(Immediate::Null);
        let instr = Instruction::new(Opcode::Store, vec![op.clone()]);
        let rt = round_trip(&simple_program(vec![instr]));
        assert_eq!(rt.functions[0].instructions[0].operands[0], op);
    }

    #[test]
    fn test_immediate_str_round_trip() {
        let op = Operand::Imm(Immediate::Str("hello, world!".into()));
        let instr = Instruction::new(Opcode::Store, vec![op.clone()]);
        let rt = round_trip(&simple_program(vec![instr]));
        assert_eq!(rt.functions[0].instructions[0].operands[0], op);
    }

    // ── Other operand kinds ───────────────────────────────────────────────────

    #[test]
    fn test_funcref_label_syscallid_round_trip() {
        let ops = vec![
            Operand::FuncRef(42),
            Operand::LabelTarget(1024),
            Operand::SyscallId(-3),
            Operand::Type(FasmType::Int32),
            Operand::Key(99),
            Operand::Required(true),
            Operand::Required(false),
        ];
        for op in ops {
            let instr = Instruction::new(Opcode::Nop, vec![op.clone()]);
            let rt = round_trip(&simple_program(vec![instr]));
            assert_eq!(rt.functions[0].instructions[0].operands[0], op);
        }
    }

    // ── Global inits ──────────────────────────────────────────────────────────

    #[test]
    fn test_global_inits_round_trip() {
        let mut prog = simple_program(vec![]);
        prog.global_inits.push(Instruction::new(
            Opcode::Reserve,
            vec![
                Operand::Key(0),
                Operand::Type(FasmType::Int32),
                Operand::Imm(Immediate::Null),
            ],
        ));
        let rt = round_trip(&prog);
        assert_eq!(rt.global_inits.len(), 1);
        assert_eq!(rt.global_inits[0].opcode, Opcode::Reserve);
    }

    // ── Function with params ──────────────────────────────────────────────────

    #[test]
    fn test_function_params_round_trip() {
        let mut prog = Program::new();
        prog.functions.push(FunctionDef {
            name: "Handler".into(),
            params: vec![
                ParamDescriptor {
                    key: 0,
                    fasm_type: FasmType::Int32,
                    name: "x".into(),
                    required: true,
                },
                ParamDescriptor {
                    key: 1,
                    fasm_type: FasmType::Bool,
                    name: "flag".into(),
                    required: false,
                },
            ],
            instructions: vec![Instruction::no_args(Opcode::Halt)],
        });
        prog.functions.push(FunctionDef {
            name: "Main".into(),
            params: vec![],
            instructions: vec![],
        });
        let rt = round_trip(&prog);
        let handler = &rt.functions[0];
        assert_eq!(handler.name, "Handler");
        assert_eq!(handler.params.len(), 2);
        assert_eq!(handler.params[0].key, 0);
        assert_eq!(handler.params[0].fasm_type, FasmType::Int32);
        assert_eq!(handler.params[0].name, "x");
        assert!(handler.params[0].required);
        assert!(!handler.params[1].required);
    }

    // ── Unknown operand tag ───────────────────────────────────────────────────

    #[test]
    fn test_unknown_operand_tag_returns_error() {
        // Build a minimal valid header + one function with one instruction
        // that has an unknown operand tag (0xEE).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.push(0x01); // version
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 global inits
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 function
                                                      // function name "Main"
        let name = b"Main";
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 0 params
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 instruction
        bytes.push(0x00); // opcode = Nop
        bytes.push(0x01); // 1 operand
        bytes.push(0xEE); // unknown tag
        assert!(decode_program(&bytes).is_err());
    }
}
