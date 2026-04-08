use serde::{Serialize, Deserialize};

/// Runtime fault codes — matches the spec's hex values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum Fault {
    NullDeref           = 0x01,
    IndexOutOfBounds    = 0x02,
    FieldNotFound       = 0x03,
    DivisionByZero      = 0x04,
    StackOverflow       = 0x05,
    UnwrapFault         = 0x06,
    WriteAccessViolation= 0x07,
    TypeMismatch        = 0x08,
    UndeclaredSlot      = 0x09,
    BadSyscall          = 0x0A,
}

impl Fault {
    pub fn code(self) -> u32 { self as u32 }

    pub fn description(self) -> &'static str {
        match self {
            Fault::NullDeref             => "NullDerefFault",
            Fault::IndexOutOfBounds      => "IndexOutOfBoundsFault",
            Fault::FieldNotFound         => "FieldNotFoundFault",
            Fault::DivisionByZero        => "DivisionByZeroFault",
            Fault::StackOverflow         => "StackOverflowFault",
            Fault::UnwrapFault           => "UnwrapFault",
            Fault::WriteAccessViolation  => "WriteAccessViolation",
            Fault::TypeMismatch          => "TypeMismatch",
            Fault::UndeclaredSlot        => "UndeclaredSlot",
            Fault::BadSyscall            => "BadSyscall",
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (0x{:02X})", self.description(), self.code())
    }
}
