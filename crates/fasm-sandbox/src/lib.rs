//! # fasm-sandbox
//!
//! Isolated execution context for FASM programs.
//!
//! Each [`Sandbox`] wraps an [`fasm_vm::executor::Executor`] with its own identity,
//! an optional [`ClockController`] to throttle instruction throughput, and a
//! syscall table that can be extended by the host at runtime.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use fasm_sandbox::Sandbox;
//!
//! let mut sandbox = Sandbox::new(/* id= */ 0);
//! sandbox.set_clock_hz(10_000); // limit to 10 000 instructions/tick (0 = unlimited)
//! sandbox.mount_syscall(100, Box::new(|args, _globals| Ok(Value::Null)));
//!
//! let result = sandbox.run(&program)?;
//! ```

pub mod sandbox;
pub mod clock;
pub mod sidecar;
pub mod plugin_manifest;

#[cfg(target_os = "linux")]
pub mod seccomp;
#[cfg(target_os = "linux")]
pub mod landlock;

pub use sandbox::{Sandbox, SandboxConfig};
pub use clock::ClockController;
pub use sidecar::SidecarPlugin;
pub use plugin_manifest::{PluginManifest, discover_auto_mount, load_manifest};

