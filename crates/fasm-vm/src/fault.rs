use serde::{Deserialize, Serialize};

/// Runtime fault codes — matches the spec's hex values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum Fault {
    NullDeref = 0x01,
    IndexOutOfBounds = 0x02,
    FieldNotFound = 0x03,
    DivisionByZero = 0x04,
    StackOverflow = 0x05,
    UnwrapFault = 0x06,
    WriteAccessViolation = 0x07,
    TypeMismatch = 0x08,
    UndeclaredSlot = 0x09,
    BadSyscall = 0x0A,
}

impl Fault {
    pub fn code(self) -> u32 {
        self as u32
    }

    pub fn description(self) -> &'static str {
        match self {
            Fault::NullDeref => "NullDerefFault",
            Fault::IndexOutOfBounds => "IndexOutOfBoundsFault",
            Fault::FieldNotFound => "FieldNotFoundFault",
            Fault::DivisionByZero => "DivisionByZeroFault",
            Fault::StackOverflow => "StackOverflowFault",
            Fault::UnwrapFault => "UnwrapFault",
            Fault::WriteAccessViolation => "WriteAccessViolation",
            Fault::TypeMismatch => "TypeMismatch",
            Fault::UndeclaredSlot => "UndeclaredSlot",
            Fault::BadSyscall => "BadSyscall",
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (0x{:02X})", self.description(), self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_codes_are_unique() {
        let faults = [
            Fault::NullDeref,
            Fault::IndexOutOfBounds,
            Fault::FieldNotFound,
            Fault::DivisionByZero,
            Fault::StackOverflow,
            Fault::UnwrapFault,
            Fault::WriteAccessViolation,
            Fault::TypeMismatch,
            Fault::UndeclaredSlot,
            Fault::BadSyscall,
        ];
        let mut codes = std::collections::HashSet::new();
        for f in &faults {
            assert!(codes.insert(f.code()), "duplicate code for {:?}", f);
        }
    }

    #[test]
    fn test_fault_code_values() {
        assert_eq!(Fault::NullDeref.code(), 0x01);
        assert_eq!(Fault::IndexOutOfBounds.code(), 0x02);
        assert_eq!(Fault::FieldNotFound.code(), 0x03);
        assert_eq!(Fault::DivisionByZero.code(), 0x04);
        assert_eq!(Fault::StackOverflow.code(), 0x05);
        assert_eq!(Fault::UnwrapFault.code(), 0x06);
        assert_eq!(Fault::WriteAccessViolation.code(), 0x07);
        assert_eq!(Fault::TypeMismatch.code(), 0x08);
        assert_eq!(Fault::UndeclaredSlot.code(), 0x09);
        assert_eq!(Fault::BadSyscall.code(), 0x0A);
    }

    #[test]
    fn test_fault_descriptions_are_nonempty() {
        let faults = [
            Fault::NullDeref,
            Fault::IndexOutOfBounds,
            Fault::FieldNotFound,
            Fault::DivisionByZero,
            Fault::StackOverflow,
            Fault::UnwrapFault,
            Fault::WriteAccessViolation,
            Fault::TypeMismatch,
            Fault::UndeclaredSlot,
            Fault::BadSyscall,
        ];
        for f in &faults {
            assert!(!f.description().is_empty(), "{:?} has empty description", f);
        }
    }

    #[test]
    fn test_fault_display_contains_code_and_description() {
        let s = format!("{}", Fault::DivisionByZero);
        assert!(s.contains("DivisionByZeroFault"), "display: {}", s);
        assert!(s.contains("0x04"), "display: {}", s);
    }
}
