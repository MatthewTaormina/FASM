//! # fasm-vm
//!
//! The FASM runtime executor: memory model, value types, instruction dispatch,
//! and fault handling.
//!
//! ## Architecture
//! Execution is driven by [`Executor`], which maintains a call stack of [`memory::Frame`]s
//! (local slots indexed by `u8`) and a single [`memory::GlobalRegister`] (indexed by `u32`).
//!
//! The main dispatch loop runs one instruction at a time, reading from the current
//! function's instruction slice and dispatching on [`fasm_bytecode::Opcode`].
//!
//! ## Key Types
//! - [`Value`] — the runtime value enum (scalars, collections, references, wrappers)
//! - [`Fault`] — runtime fault codes that trigger `TRY`/`CATCH` or terminate execution
//! - [`memory::Frame`] — per-call local slot storage
//! - [`memory::GlobalRegister`] — shared global slot storage
//! - [`executor::Executor`] — the VM dispatch loop and syscall table
//! - [`executor::SyscallFn`] — the signature for host-provided syscall handlers

pub mod value;
pub mod fault;
pub mod memory;
pub mod executor;

pub use value::Value;
pub use fault::Fault;
pub use memory::{Frame, GlobalRegister};
pub use executor::Executor;
