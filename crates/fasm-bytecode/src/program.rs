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
    /// All function definitions including Main.
    pub functions: Vec<FunctionDef>,
}

impl Program {
    pub fn new() -> Self {
        Self {
            version: 0x01,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{instruction::Operand, Instruction, Opcode};

    fn make_program_with_functions(names: &[&str]) -> Program {
        let mut prog = Program::new();
        for &name in names {
            prog.functions.push(FunctionDef {
                name: name.to_string(),
                params: vec![],
                instructions: vec![Instruction::no_args(Opcode::Halt)],
            });
        }
        prog
    }

    #[test]
    fn test_new_program_defaults() {
        let prog = Program::new();
        assert_eq!(prog.version, 0x01);
        assert!(prog.functions.is_empty());
    }

    #[test]
    fn test_default_equals_new() {
        let a = Program::new();
        let b = Program::default();
        assert_eq!(a.version, b.version);
        assert_eq!(a.functions.len(), b.functions.len());
    }

    #[test]
    fn test_get_function_found() {
        let prog = make_program_with_functions(&["Main", "Helper"]);
        assert!(prog.get_function("Main").is_some());
        assert!(prog.get_function("Helper").is_some());
    }

    #[test]
    fn test_get_function_not_found() {
        let prog = make_program_with_functions(&["Main"]);
        assert!(prog.get_function("Missing").is_none());
    }

    #[test]
    fn test_get_function_index() {
        let prog = make_program_with_functions(&["Main", "A", "B"]);
        assert_eq!(prog.get_function_index("Main"), Some(0));
        assert_eq!(prog.get_function_index("A"), Some(1));
        assert_eq!(prog.get_function_index("B"), Some(2));
        assert_eq!(prog.get_function_index("X"), None);
    }

    #[test]
    fn test_instruction_constructors() {
        let instr = Instruction::no_args(Opcode::Halt);
        assert_eq!(instr.opcode, Opcode::Halt);
        assert!(instr.operands.is_empty());

        let instr2 = Instruction::new(
            Opcode::Add,
            vec![
                Operand::Slot(crate::instruction::SlotRef::Local(0)),
                Operand::Slot(crate::instruction::SlotRef::Local(1)),
                Operand::Slot(crate::instruction::SlotRef::Local(2)),
            ],
        );
        assert_eq!(instr2.opcode, Opcode::Add);
        assert_eq!(instr2.operands.len(), 3);
    }
}
