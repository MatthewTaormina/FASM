use fasm_bytecode::types::FasmType;

/// Top-level program AST.
#[derive(Debug, Default)]
pub struct ProgramAst {
    pub defines: Vec<Define>,
    pub imports: Vec<Import>,
    pub global_reserves: Vec<GlobalReserve>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone)]
pub struct Define {
    pub name: String,
    pub value: AstValue,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub path: String,
    pub alias: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct GlobalReserve {
    pub index: u32,
    pub fasm_type: FasmType,
    pub init: AstValue,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub key: AstValue,   // u32 or DEFINE name
    pub fasm_type: FasmType,
    pub name: String,
    pub required: bool,
    pub line: usize,
}

/// A statement inside a function body.
#[derive(Debug, Clone)]
pub enum Statement {
    Local(LocalDecl),
    Label(String, usize),
    Instr(Instr),
    TryBlock { catch_label: String, end_label: String, body: Vec<Statement>, catch_body: Vec<Statement>, line: usize },
}

#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub index: u8,
    pub fasm_type: FasmType,
    pub name: String,
    pub line: usize,
}

/// A single instruction with its operand expressions.
#[derive(Debug, Clone)]
pub struct Instr {
    pub mnemonic: String,
    pub operands: Vec<AstValue>,
    pub line: usize,
}

/// An operand expression before resolution.
#[derive(Debug, Clone)]
pub enum AstValue {
    Ident(String),       // symbolic name, DEFINE, or keyword
    Integer(i64),
    HexInt(u64),
    Float(f64),
    Deref(String),       // &name
    Str(String),         // "..." literal — compiled to VEC<UINT8> (UTF-8 bytes)
    Null,
    True,
    False,
}

impl AstValue {
    pub fn as_ident(&self) -> Option<&str> {
        if let AstValue::Ident(s) = self { Some(s) } else { None }
    }
}
