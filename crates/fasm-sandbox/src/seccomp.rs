//! Seccomp-BPF syscall denylist for FASM execution threads.
//!
//! Calling [`apply_denylist`] in the execution thread installs a BPF filter
//! that blocks the subset of Linux syscalls most likely to be abused for
//! sandbox escapes, while allowing everything the FASM VM interpreter needs to
//! run normally (memory allocation, mutexes, I/O on existing file descriptors,
//! etc.).
//!
//! The filter is applied only to the calling thread (no `SECCOMP_FILTER_FLAG_TSYNC`),
//! so other Tokio async-executor threads are unaffected.  Because Tokio reuses
//! `spawn_blocking` threads, a thread-local flag ensures the filter is installed
//! at most once per OS thread.
//!
//! # Blocked syscalls
//!
//! | Category              | Syscalls blocked                                     |
//! |-----------------------|------------------------------------------------------|
//! | Process creation      | `fork`, `vfork`, `clone`, `clone3`, `execve`, `execveat` |
//! | Networking            | `socket`, `socketpair`, `bind`, `connect`, `listen`, `accept`, `accept4` |
//! | Privilege escalation  | `setuid`, `setgid`, `setreuid`, `setregid`, `setresuid`, `setresgid` |
//! | Filesystem mounting   | `mount`, `umount2`, `pivot_root`, `chroot`          |
//! | Kernel patching       | `init_module`, `finit_module`, `delete_module`, `kexec_load`, `kexec_file_load` |
//! | Namespace escape      | `unshare`, `setns`                                   |
//! | Process introspection | `ptrace`, `process_vm_readv`, `process_vm_writev`   |
//! | BPF (self-escalation) | `bpf`                                                |
//! | Perf (side-channel)   | `perf_event_open`                                    |
//! | Handle-based open     | `open_by_handle_at`                                  |

use std::cell::Cell;

thread_local! {
    /// Set to `true` once a seccomp filter has been installed on this OS thread.
    static APPLIED: Cell<bool> = const { Cell::new(false) };
}

// ── BPF instruction helpers ───────────────────────────────────────────────────

/// x86-64 architecture constant used in the `seccomp_data.arch` field.
///
/// Value: `AUDIT_ARCH_X86_64 = EM_X86_64 | __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE`
///       = 62 | 0x80000000 | 0x40000000 = 0xc000003e
const AUDIT_ARCH_X86_64: u32 = 0xc000003e;

/// Byte offset of the `nr` (syscall number) field in `struct seccomp_data`.
const OFF_NR: u32 = 0;
/// Byte offset of the `arch` field in `struct seccomp_data`.
const OFF_ARCH: u32 = 4;

/// Build a BPF "statement" instruction (jt=0, jf=0).
#[inline]
fn bpf_stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt: 0, jf: 0, k }
}

/// Build a BPF conditional-jump instruction.
///
/// * `jt` — instructions to skip when the condition is **true**.
/// * `jf` — instructions to skip when the condition is **false**.
#[inline]
fn bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

// ── BPF opcode constants ──────────────────────────────────────────────────────

const BPF_LD_W_ABS: u16  = (libc::BPF_LD  | libc::BPF_W   | libc::BPF_ABS) as u16;
const BPF_JMP_JEQ_K: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K)   as u16;
const BPF_RET_K: u16     = (libc::BPF_RET | libc::BPF_K)                    as u16;

// ── Seccomp return values ─────────────────────────────────────────────────────

const SECCOMP_RET_ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
/// Return EPERM (1) to the calling thread for denied syscalls.
const SECCOMP_RET_EPERM: u32 = libc::SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff);

// ── Syscall numbers (x86-64 Linux) ───────────────────────────────────────────

const SYS_FORK: u32            = 57;
const SYS_VFORK: u32           = 58;
const SYS_EXECVE: u32          = 59;
const SYS_CLONE: u32           = 56;
const SYS_CLONE3: u32          = 435;
const SYS_EXECVEAT: u32        = 322;
const SYS_SOCKET: u32          = 41;
const SYS_SOCKETPAIR: u32      = 53;
const SYS_BIND: u32            = 49;
const SYS_CONNECT: u32         = 42;
const SYS_LISTEN: u32          = 50;
const SYS_ACCEPT: u32          = 43;
const SYS_ACCEPT4: u32         = 288;
const SYS_PTRACE: u32          = 101;
const SYS_PROCESS_VM_READV: u32  = 310;
const SYS_PROCESS_VM_WRITEV: u32 = 311;
const SYS_MOUNT: u32           = 165;
const SYS_UMOUNT2: u32         = 166;
const SYS_PIVOT_ROOT: u32      = 155;
const SYS_CHROOT: u32          = 161;
const SYS_INIT_MODULE: u32     = 175;
const SYS_FINIT_MODULE: u32    = 313;
const SYS_DELETE_MODULE: u32   = 176;
const SYS_KEXEC_LOAD: u32      = 246;
const SYS_KEXEC_FILE_LOAD: u32 = 320;
const SYS_UNSHARE: u32         = 272;
const SYS_SETNS: u32           = 308;
const SYS_SETUID: u32          = 105;
const SYS_SETGID: u32          = 106;
const SYS_SETREUID: u32        = 113;
const SYS_SETREGID: u32        = 114;
const SYS_SETRESUID: u32       = 117;
const SYS_SETRESGID: u32       = 119;
const SYS_BPF: u32             = 321;
const SYS_PERF_EVENT_OPEN: u32 = 298;
const SYS_OPEN_BY_HANDLE_AT: u32 = 304;

/// All syscall numbers that will receive `EPERM`.
const DENIED_SYSCALLS: &[u32] = &[
    SYS_FORK,
    SYS_VFORK,
    SYS_CLONE,
    SYS_CLONE3,
    SYS_EXECVE,
    SYS_EXECVEAT,
    SYS_SOCKET,
    SYS_SOCKETPAIR,
    SYS_BIND,
    SYS_CONNECT,
    SYS_LISTEN,
    SYS_ACCEPT,
    SYS_ACCEPT4,
    SYS_PTRACE,
    SYS_PROCESS_VM_READV,
    SYS_PROCESS_VM_WRITEV,
    SYS_MOUNT,
    SYS_UMOUNT2,
    SYS_PIVOT_ROOT,
    SYS_CHROOT,
    SYS_INIT_MODULE,
    SYS_FINIT_MODULE,
    SYS_DELETE_MODULE,
    SYS_KEXEC_LOAD,
    SYS_KEXEC_FILE_LOAD,
    SYS_UNSHARE,
    SYS_SETNS,
    SYS_SETUID,
    SYS_SETGID,
    SYS_SETREUID,
    SYS_SETREGID,
    SYS_SETRESUID,
    SYS_SETRESGID,
    SYS_BPF,
    SYS_PERF_EVENT_OPEN,
    SYS_OPEN_BY_HANDLE_AT,
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Install a seccomp-BPF syscall denylist on the **calling thread**.
///
/// This is idempotent per OS thread: if the filter has already been installed
/// on this thread (tracked via a thread-local), the call returns `Ok(())` immediately.
///
/// # Errors
/// Returns an error string if `prctl` fails.  This can happen when:
/// - the kernel was compiled without seccomp support (`ENOSYS` / `EINVAL`), or
/// - the process is in a user namespace that disallows filter installation.
///
/// Callers should treat errors as non-fatal and log them, not abort execution.
pub fn apply_denylist() -> Result<(), String> {
    // Only apply once per OS thread.
    if APPLIED.with(|c| c.get()) {
        return Ok(());
    }

    // `mut` so we can call `as_mut_ptr()` for the sock_fprog field.
    let mut filter = build_filter();

    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        // Safety: the Vec is alive for the duration of both prctl calls below.
        filter: filter.as_mut_ptr(),
    };

    unsafe {
        // PR_SET_NO_NEW_PRIVS is required before installing a seccomp filter
        // without CAP_SYS_ADMIN.
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(format!(
                "prctl(PR_SET_NO_NEW_PRIVS) failed: errno={}",
                *libc::__errno_location()
            ));
        }

        if libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &prog as *const libc::sock_fprog as libc::c_ulong,
            0,
            0,
        ) != 0
        {
            return Err(format!(
                "prctl(PR_SET_SECCOMP, FILTER) failed: errno={}",
                *libc::__errno_location()
            ));
        }
    }

    APPLIED.with(|c| c.set(true));
    Ok(())
}

// ── Filter construction ───────────────────────────────────────────────────────

/// Build the BPF program for the denylist filter.
fn build_filter() -> Vec<libc::sock_filter> {
    // Each denied syscall costs 2 instructions (JEQ + RET_EPERM).
    // Preamble: 3 instructions (LOAD arch, JEQ, RET_KILL if wrong arch).
    // Postamble: 2 instructions (LOAD nr, default ALLOW, default RET_EPERM). <- actually 1 ALLOW at end
    // Structure per denied syscall:
    //   LOAD nr  <- done once at position 3
    //   JEQ(denied_nr, 0, 1) <- if match: continue to DENY; else skip DENY
    //   RET EPERM            <- deny
    // Final: RET ALLOW

    let n = DENIED_SYSCALLS.len();
    let mut insns: Vec<libc::sock_filter> = Vec::with_capacity(4 + n * 2);

    // ── Preamble: architecture check ─────────────────────────────────────────
    // 0: Load arch field
    insns.push(bpf_stmt(BPF_LD_W_ABS, OFF_ARCH));
    // 1: If arch == AUDIT_ARCH_X86_64: skip 1 (to LOAD nr); else fall through to KILL
    insns.push(bpf_jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0));
    // 2: Wrong architecture — kill the thread immediately
    insns.push(bpf_stmt(BPF_RET_K, libc::SECCOMP_RET_KILL_THREAD));

    // ── Body: load syscall number, then check each denied entry ──────────────
    // 3: Load syscall number (nr field at offset 0)
    insns.push(bpf_stmt(BPF_LD_W_ABS, OFF_NR));

    // For each denied syscall:
    //   JEQ(nr, jt=0, jf=1) — match → fall to DENY; no-match → skip DENY
    //   RET EPERM
    for &nr in DENIED_SYSCALLS {
        insns.push(bpf_jump(BPF_JMP_JEQ_K, nr, 0, 1));
        insns.push(bpf_stmt(BPF_RET_K, SECCOMP_RET_EPERM));
    }

    // ── Postamble: default allow ──────────────────────────────────────────────
    insns.push(bpf_stmt(BPF_RET_K, SECCOMP_RET_ALLOW));

    insns
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_non_empty() {
        let f = build_filter();
        assert!(!f.is_empty(), "filter must have at least one instruction");
        // Preamble (3) + LOAD nr (1) + 2*DENIED + ALLOW (1)
        let expected = 3 + 1 + DENIED_SYSCALLS.len() * 2 + 1;
        assert_eq!(f.len(), expected, "unexpected filter length");
    }

    #[test]
    fn apply_denylist_succeeds_on_linux() {
        // We apply the filter in a dedicated thread so it doesn't pollute
        // the main test thread's seccomp state.
        let handle = std::thread::spawn(apply_denylist);
        let result = handle.join().expect("thread panicked");
        assert!(result.is_ok(), "apply_denylist failed: {:?}", result);
    }

    #[test]
    fn apply_is_idempotent() {
        // Calling apply_denylist twice on the same thread must not error.
        let handle = std::thread::spawn(|| {
            apply_denylist().and_then(|_| apply_denylist())
        });
        assert!(handle.join().expect("thread panicked").is_ok());
    }

    /// After the denylist is applied, `fork()` must return -1 with EPERM.
    #[test]
    fn test_fork_blocked_by_denylist() {
        let result = std::thread::spawn(|| -> Result<(), String> {
            apply_denylist()?;
            let ret = unsafe { libc::fork() };
            if ret == -1 {
                let errno = unsafe { *libc::__errno_location() };
                if errno == libc::EPERM {
                    return Ok(());
                }
                return Err(format!("fork returned unexpected errno {}", errno));
            }
            // fork somehow succeeded – clean up the child.
            if ret == 0 {
                unsafe { libc::_exit(0) };
            }
            Err("fork was not blocked by the seccomp denylist".into())
        })
        .join()
        .expect("thread panicked");
        assert!(result.is_ok(), "{:?}", result);
    }
}
