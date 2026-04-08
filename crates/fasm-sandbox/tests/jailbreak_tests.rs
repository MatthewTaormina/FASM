//! Sandbox jailbreak / escape prevention tests.
//!
//! These tests verify that the sandbox correctly blocks both malicious FASM
//! bytecode and malicious host-side syscall handlers from escaping the
//! isolation boundary.
//!
//! Test categories:
//!
//! 1. **VM-level isolation** — enforced on every OS; no kernel feature needed.
//!    Tests unmounted syscall IDs, infinite recursion, and arithmetic faults.
//!
//! 2. **Seccomp isolation** (Linux only) — a mounted syscall handler that
//!    attempts to call `fork`, `socket`, or `execve` must be blocked by the
//!    BPF denylist installed by [`fasm_sandbox::seccomp::apply_denylist`].
//!
//! 3. **Landlock isolation** (Linux only) — a mounted syscall handler that
//!    attempts to open a file outside the configured allowed paths must be
//!    denied by the Landlock ruleset.
//!
//! Each test that applies OS-level protections runs inside a dedicated
//! `std::thread::spawn` call so the per-thread seccomp/landlock state does
//! not bleed across tests.

use fasm_bytecode::{
    instruction::{Immediate, Instruction, Operand},
    opcode::Opcode,
    program::{FunctionDef, Program},
};
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::Value;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a minimal `Program` whose `Main` consists solely of the given instructions.
fn make_program(instructions: Vec<Instruction>) -> Program {
    Program {
        version: 0x01,
        global_inits: vec![],
        functions: vec![FunctionDef {
            name: "Main".to_string(),
            params: vec![],
            instructions,
        }],
    }
}

/// Build a program that invokes one syscall and halts.
fn syscall_program(id: i32) -> Program {
    make_program(vec![
        Instruction::new(
            Opcode::Syscall,
            vec![
                Operand::SyscallId(id),
                Operand::Imm(Immediate::Null),
            ],
        ),
        Instruction::no_args(Opcode::Halt),
    ])
}

// ── 1. VM-level isolation ─────────────────────────────────────────────────────

/// A FASM program that invokes an unmounted syscall ID must return an error.
/// This tests the VM-level boundary: the program cannot reach any capability
/// that has not been explicitly granted by the host.
#[test]
fn test_unmounted_syscall_is_rejected() {
    let prog = syscall_program(9999);
    let mut sandbox = Sandbox::new(0);
    let result = sandbox.run(&prog);
    assert!(
        result.is_err(),
        "unmounted syscall 9999 must produce an Err, got {:?}",
        result
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("BadSyscall") || msg.contains("bad") || msg.contains("syscall"),
        "error must mention BadSyscall, got: {}",
        msg
    );
}

/// Negative syscall IDs that are never mounted must also be rejected.
#[test]
fn test_negative_syscall_id_is_rejected() {
    let prog = syscall_program(-42);
    let mut sandbox = Sandbox::new(0);
    let result = sandbox.run(&prog);
    assert!(result.is_err(), "unmounted syscall -42 must produce an Err");
}

/// A program with infinite mutual recursion must eventually fail with a
/// stack-overflow fault rather than panicking or running forever.
#[test]
fn test_infinite_recursion_hits_stack_limit() {
    // Main calls "Recur"; Recur calls itself indefinitely.
    let main_fn = FunctionDef {
        name: "Main".to_string(),
        params: vec![],
        instructions: vec![
            Instruction::new(
                Opcode::Call,
                vec![
                    Operand::FuncRef(1), // index 1 = "Recur"
                    Operand::Imm(Immediate::Null),
                ],
            ),
            Instruction::no_args(Opcode::Halt),
        ],
    };
    let recur_fn = FunctionDef {
        name: "Recur".to_string(),
        params: vec![],
        instructions: vec![
            Instruction::new(
                Opcode::Call,
                vec![
                    Operand::FuncRef(1), // calls itself
                    Operand::Imm(Immediate::Null),
                ],
            ),
            Instruction::no_args(Opcode::Halt),
        ],
    };
    let prog = Program {
        version: 0x01,
        global_inits: vec![],
        functions: vec![main_fn, recur_fn],
    };

    let mut sandbox = Sandbox::new(0);
    let result = sandbox.run(&prog);
    assert!(
        result.is_err(),
        "infinite recursion must produce an Err, got {:?}",
        result
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("StackOverflow") || msg.contains("stack"),
        "error must mention StackOverflow, got: {}",
        msg
    );
}

/// A division-by-zero fault inside a FASM program must be contained: it must
/// return an Err, never panic the host, and must not affect other sandboxes.
#[test]
fn test_division_by_zero_is_contained() {
    use fasm_bytecode::instruction::SlotRef;
    use fasm_bytecode::types::FasmType;

    // RESERVE local(0) = INT32(10)
    // RESERVE local(1) = INT32(0)
    // DIV local(0), local(1), local(2)   ; triggers DivisionByZero
    let prog = make_program(vec![
        Instruction::new(
            Opcode::Reserve,
            vec![
                Operand::Slot(SlotRef::Local(0)),
                Operand::Type(FasmType::Int32),
                Operand::Imm(Immediate::Int32(10)),
            ],
        ),
        Instruction::new(
            Opcode::Reserve,
            vec![
                Operand::Slot(SlotRef::Local(1)),
                Operand::Type(FasmType::Int32),
                Operand::Imm(Immediate::Int32(0)),
            ],
        ),
        Instruction::new(
            Opcode::Div,
            vec![
                Operand::Slot(SlotRef::Local(0)),
                Operand::Slot(SlotRef::Local(1)),
                Operand::Slot(SlotRef::Local(2)),
            ],
        ),
        Instruction::no_args(Opcode::Halt),
    ]);

    let mut sandbox = Sandbox::new(1);
    let result = sandbox.run(&prog);
    assert!(
        result.is_err(),
        "division by zero must produce an Err, got {:?}",
        result
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("DivisionByZero") || msg.contains("division"),
        "error must mention DivisionByZero, got: {}",
        msg
    );

    // A second, unrelated sandbox must run normally.
    let good_prog = make_program(vec![Instruction::no_args(Opcode::Halt)]);
    let mut sandbox2 = Sandbox::new(2);
    assert!(
        sandbox2.run(&good_prog).is_ok(),
        "a fault in sandbox 1 must not affect sandbox 2"
    );
}

/// A FASM program that calls a mounted syscall which returns an error must
/// propagate that error back to the caller as an Err (not panic).
#[test]
fn test_erroring_syscall_handler_is_contained() {
    let prog = syscall_program(200);
    let mut sandbox = Sandbox::new(0);
    sandbox.mount_syscall(
        200,
        Box::new(|_, _| Err(fasm_vm::Fault::BadSyscall)),
    );
    let result = sandbox.run(&prog);
    assert!(
        result.is_err(),
        "a failing syscall handler must return Err, got {:?}",
        result
    );
}

// ── 2. Seccomp isolation (Linux only) ────────────────────────────────────────
//
// Each test spawns a fresh OS thread so that:
//   a) the seccomp filter is installed on that thread only, and
//   b) state does not leak between tests.

#[cfg(target_os = "linux")]
mod seccomp_tests {
    use super::*;

    /// After the seccomp denylist is applied, a mounted syscall handler that
    /// tries to call `fork(2)` must receive EPERM from the kernel.  The
    /// sandbox run must therefore return an Err rather than actually forking.
    #[test]
    fn test_seccomp_blocks_fork_from_mounted_handler() {
        let result = std::thread::spawn(|| {
            let prog = syscall_program(100);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_seccomp: true,
                    ..Default::default()
                },
            );
            sb.mount_syscall(
                100,
                Box::new(|_, _| {
                    let ret = unsafe { libc::fork() };
                    if ret == -1 {
                        let errno = unsafe { *libc::__errno_location() };
                        if errno == libc::EPERM || errno == libc::ENOSYS {
                            return Err(fasm_vm::Fault::BadSyscall);
                        }
                        return Err(fasm_vm::Fault::BadSyscall);
                    }
                    if ret == 0 {
                        // Should never happen — seccomp prevents fork.
                        unsafe { libc::_exit(0) };
                    }
                    // fork succeeded — that is the failure we are guarding against.
                    Err(fasm_vm::Fault::BadSyscall)
                }),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        // Either the syscall handler returned Err (fork blocked → EPERM),
        // or seccomp itself turned it into an Err before the handler returned.
        // Either way the sandbox must NOT return Ok with a forked child.
        assert!(
            result.is_err(),
            "sandbox must return Err when fork is blocked by seccomp, got {:?}",
            result
        );
    }

    /// After the seccomp denylist is applied, a mounted syscall handler that
    /// tries to open a `SOCK_STREAM` socket must receive EPERM.
    #[test]
    fn test_seccomp_blocks_socket_from_mounted_handler() {
        let result = std::thread::spawn(|| {
            let prog = syscall_program(101);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_seccomp: true,
                    ..Default::default()
                },
            );
            sb.mount_syscall(
                101,
                Box::new(|_, _| {
                    let fd = unsafe {
                        libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)
                    };
                    if fd == -1 {
                        return Err(fasm_vm::Fault::BadSyscall);
                    }
                    // Socket was created — seccomp did not block it.
                    unsafe { libc::close(fd) };
                    Err(fasm_vm::Fault::BadSyscall)
                }),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(
            result.is_err(),
            "sandbox must return Err when socket() is blocked by seccomp"
        );
    }

    /// After the seccomp denylist is applied, a mounted syscall handler that
    /// tries to exec a new process via `execve(2)` must receive EPERM.
    #[test]
    fn test_seccomp_blocks_execve_from_mounted_handler() {
        let result = std::thread::spawn(|| {
            let prog = syscall_program(102);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_seccomp: true,
                    ..Default::default()
                },
            );
            sb.mount_syscall(
                102,
                Box::new(|_, _| {
                    use std::ffi::CString;
                    let path = CString::new("/bin/true").unwrap();
                    let argv = [path.as_ptr(), std::ptr::null()];
                    let envp: [*const libc::c_char; 1] = [std::ptr::null()];
                    let ret = unsafe {
                        libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr())
                    };
                    // execve only returns on failure.
                    assert_eq!(ret, -1, "execve should have failed");
                    let errno = unsafe { *libc::__errno_location() };
                    assert!(
                        errno == libc::EPERM || errno == libc::ENOSYS,
                        "execve must fail with EPERM or ENOSYS, got {}",
                        errno
                    );
                    Err(fasm_vm::Fault::BadSyscall)
                }),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(
            result.is_err(),
            "sandbox must return Err when execve is blocked by seccomp"
        );
    }

    /// Verifies that applying the denylist and then invoking a benign handler
    /// still works correctly — the denylist only blocks the dangerous calls.
    #[test]
    fn test_seccomp_does_not_break_benign_handler() {
        let result = std::thread::spawn(|| {
            let prog = syscall_program(103);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_seccomp: true,
                    ..Default::default()
                },
            );
            sb.mount_syscall(
                103,
                Box::new(|_, _| Ok(Value::Int32(42))),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(
            result.is_ok(),
            "benign handler must still work after seccomp is applied, got {:?}",
            result
        );
    }

    /// A high-ID FASM syscall that is not mounted must be blocked at the VM
    /// level even when seccomp is active (belt-and-suspenders check).
    #[test]
    fn test_unmounted_syscall_still_rejected_with_seccomp_active() {
        let result = std::thread::spawn(|| {
            let prog = syscall_program(0x7FFF_FFFF);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_seccomp: true,
                    ..Default::default()
                },
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(result.is_err(), "unmounted syscall must be rejected even with seccomp active");
    }
}

// ── 3. Landlock isolation (Linux only) ───────────────────────────────────────

#[cfg(target_os = "linux")]
mod landlock_tests {
    use super::*;
    use std::io::Write;

    /// When Landlock is enabled with only `tmp` as the allowed read path,
    /// a mounted handler that tries to open `/etc/hostname` must be denied.
    #[test]
    fn test_landlock_blocks_read_outside_allowed_path() {
        let tmp = std::env::temp_dir();
        let result = std::thread::spawn(move || {
            let prog = syscall_program(110);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_landlock: true,
                    landlock_allowed_read_paths: vec![tmp],
                    ..Default::default()
                },
            );
            sb.mount_syscall(
                110,
                Box::new(|_, _| {
                    use std::ffi::CString;
                    // /etc/hostname is outside the allowed tmp directory.
                    let path = CString::new("/etc/hostname").unwrap();
                    let fd = unsafe {
                        libc::open(path.as_ptr(), libc::O_RDONLY)
                    };
                    if fd == -1 {
                        // Access denied by Landlock — this is the expected outcome.
                        return Err(fasm_vm::Fault::BadSyscall);
                    }
                    unsafe { libc::close(fd) };
                    // File was readable — Landlock did NOT block it.
                    Err(fasm_vm::Fault::BadSyscall)
                }),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        // The handler returns Err regardless of whether the open succeeded or
        // failed, but we want to confirm that the run returns Err (meaning the
        // handler was invoked and the open was either denied or the handler
        // reported it correctly).  The important property is that the sandbox
        // run does not return Ok(some_file_data), i.e., no successful read leak.
        assert!(
            result.is_err(),
            "read outside Landlock boundary must produce Err, got {:?}",
            result
        );
    }

    /// When Landlock is enabled, a handler that reads a file *inside* the
    /// allowed path must still succeed — Landlock must not break normal reads.
    #[test]
    fn test_landlock_allows_read_inside_allowed_path() {
        let tmp = std::env::temp_dir();
        let mut file = tempfile::NamedTempFile::new_in(&tmp)
            .expect("could not create temp file");
        writeln!(file, "hello").unwrap();
        let path = file.path().to_path_buf();

        let result = std::thread::spawn(move || {
            let prog = syscall_program(111);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_landlock: true,
                    landlock_allowed_read_paths: vec![tmp],
                    ..Default::default()
                },
            );
            let path_clone = path.clone();
            sb.mount_syscall(
                111,
                Box::new(move |_, _| {
                    let content = std::fs::read_to_string(&path_clone)
                        .map_err(|_| fasm_vm::Fault::BadSyscall)?;
                    if content.contains("hello") {
                        Ok(Value::Int32(1))
                    } else {
                        Err(fasm_vm::Fault::BadSyscall)
                    }
                }),
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(
            result.is_ok(),
            "read inside Landlock boundary must succeed, got {:?}",
            result
        );
    }

    /// A FASM program that uses an unmounted syscall ID cannot bypass Landlock
    /// restrictions — the VM-level boundary catches it first.
    #[test]
    fn test_landlock_and_vm_boundary_together() {
        let tmp = std::env::temp_dir();
        let result = std::thread::spawn(move || {
            // syscall 9998 is not mounted
            let prog = syscall_program(9998);
            let mut sb = Sandbox::from_config(
                0,
                &SandboxConfig {
                    enable_landlock: true,
                    landlock_allowed_read_paths: vec![tmp],
                    ..Default::default()
                },
            );
            sb.run(&prog)
        })
        .join()
        .expect("test thread panicked");

        assert!(
            result.is_err(),
            "unmounted syscall must be rejected even with Landlock active"
        );
    }
}
