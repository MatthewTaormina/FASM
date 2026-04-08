use crate::{FasmType, Instruction};

/// Describes a single PARAM declaration on a function.
#[derive(Debug, Clone)]
pub struct ParamDescriptor {
    pub key: u32,
    pub fasm_type: FasmType,
    pub name: String,
    pub required: bool,
}

/// A compiled function body.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<ParamDescriptor>,
    pub instructions: Vec<Instruction>,
}

/// A fully compiled FASM program, ready for the VM.
#[derive(Debug, Clone)]
pub struct Program {
    /// Version byte, should be 0x01 for this implementation.
    pub version: u8,
    /// Global RESERVE instructions run before Main.
    pub global_inits: Vec<Instruction>,
    /// All function definitions including Main.
    pub functions: Vec<FunctionDef>,
}

impl Program {
    pub fn new() -> Self {
        Self {
            version: 0x01,
            global_inits: vec![],
            functions: vec![],
        }
    }

    pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.iter().find(|f| f.name == name)
    }

    pub fn get_function_index(&self, name: &str) -> Option<usize> {
        self.functions.iter().position(|f| f.name == name)
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}
