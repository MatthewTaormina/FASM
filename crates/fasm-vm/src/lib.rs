//! # fasm-vm
//!
//! The FASM runtime executor: memory model, value types, instruction dispatch,
//! and fault handling.
//!
//! ## Architecture
//! Execution is driven by [`Executor`], which maintains a call stack of [`memory::Frame`]s
//! (local slots indexed by `u8`).  All state is local to the call frame — there is no
//! shared global register.  Between-invocation state is passed explicitly via the $args
//! struct using the `KEY_STATE` convention.
//!
//! The main dispatch loop runs one instruction at a time, reading from the current
//! function's instruction slice and dispatching on [`fasm_bytecode::Opcode`].
//!
//! ## Key Types
//! - [`Value`] — the runtime value enum (scalars, collections, references, wrappers)
//! - [`Fault`] — runtime fault codes that trigger `TRY`/`CATCH` or terminate execution
//! - [`memory::Frame`] — per-call local slot storage
//! - [`executor::Executor`] — the VM dispatch loop and syscall table
//! - [`executor::SyscallFn`] — the signature for host-provided syscall handlers

pub mod executor;
pub mod fault;
pub mod memory;
pub mod value;

pub use executor::{Executor, JitDispatcher};
pub use fault::Fault;
pub use memory::Frame;
pub use value::Value;
