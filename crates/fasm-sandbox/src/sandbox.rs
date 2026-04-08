use fasm_bytecode::Program;
use fasm_vm::{Executor, Value};
use fasm_vm::executor::SyscallFn;
use crate::clock::ClockController;

/// An isolated execution context for a single FASM program.
pub struct Sandbox {
    pub id: u64,
    executor: Executor,
    pub clock: ClockController,
}

impl Sandbox {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            executor: Executor::new(),
            clock: ClockController::new(),
        }
    }

    /// Set instructions-per-tick limit (0 = unlimited).
    pub fn set_clock_hz(&mut self, hz: u64) {
        self.clock.instructions_per_tick = hz;
    }

    /// Mount a custom syscall handler.
    pub fn mount_syscall(&mut self, id: i32, handler: SyscallFn) {
        self.executor.mount_syscall(id, handler);
    }

    /// Mount an IPC sidecar process as a Syscall.
    pub fn mount_sidecar(&mut self, id: i32, cmd: &str, args: &[&str]) {
        use std::sync::{Arc, Mutex};
        let sidecar = crate::sidecar::SidecarPlugin::new(cmd, args);
        let locked = Arc::new(Mutex::new(sidecar));
        
        self.mount_syscall(id, Box::new(move |val, _| {
            let mut plg = locked.lock().unwrap();
            plg.call(&val)
        }));
    }

    /// Run the program to completion from Main.
    pub fn run(&mut self, program: &Program) -> Result<Value, String> {
        self.executor.run(program)
    }
}
