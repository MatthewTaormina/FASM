//! Seccomp-BPF syscall denylist for FASM execution threads.
//!
//! Uses the [`seccompiler`] crate — the same library used by Firecracker — to
//! build and install a BPF filter that blocks high-risk syscalls (process
//! creation, networking, ptrace, kernel-module loading, etc.) while allowing
//! everything the Rust runtime and the FASM interpreter legitimately need.
//!
//! The filter is installed on the calling OS thread only (no `TSYNC` flag),
//! so other Tokio threads are unaffected.  A thread-local flag prevents
//! double-installation when Tokio reuses `spawn_blocking` worker threads.
//!
//! # Denied syscalls
//!
//! | Category              | Syscalls                                                  |
//! |-----------------------|-----------------------------------------------------------|
//! | Process creation      | `fork`, `vfork`, `clone`, `clone3`, `execve`, `execveat` |
//! | Networking            | `socket`, `socketpair`, `bind`, `connect`, `listen`, `accept`, `accept4` |
//! | Filesystem mounting   | `mount`, `umount2`, `pivot_root`, `chroot`                |
//! | Kernel patching       | `init_module`, `finit_module`, `delete_module`, `kexec_load`, `kexec_file_load` |
//! | Namespace escape      | `unshare`, `setns`                                        |
//! | Process introspection | `ptrace`, `process_vm_readv`, `process_vm_writev`         |
//! | BPF self-escalation   | `bpf`                                                     |
//! | Perf side-channel     | `perf_event_open`                                         |
//! | Handle-based open     | `open_by_handle_at`                                       |

use std::cell::Cell;
use std::collections::BTreeMap;

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch};

thread_local! {
    static SECCOMP_APPLIED: Cell<bool> = const { Cell::new(false) };
}

/// Build the list of syscalls to deny for the current target architecture.
///
/// Returns `(arch, denied_syscalls)`.  The syscall numbers come from the
/// `libc` crate which already maps them per architecture, so the same source
/// code compiles correctly on x86_64, aarch64, and riscv64.
fn denylist() -> (TargetArch, Vec<i64>) {
    // Determine the target architecture for the seccomp filter.
    #[cfg(target_arch = "x86_64")]
    let arch = TargetArch::x86_64;
    #[cfg(target_arch = "aarch64")]
    let arch = TargetArch::aarch64;
    #[cfg(target_arch = "riscv64")]
    let arch = TargetArch::riscv64;

    // Common syscalls present on all supported Linux architectures.
    let mut denied: Vec<i64> = vec![
        libc::SYS_execve,
        libc::SYS_clone,
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_accept,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_socketpair,
        libc::SYS_ptrace,
        libc::SYS_mount,
        libc::SYS_chroot,
        libc::SYS_init_module,
        libc::SYS_delete_module,
        libc::SYS_kexec_load,
        libc::SYS_unshare,
        libc::SYS_accept4,
        libc::SYS_perf_event_open,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_setns,
        libc::SYS_execveat,
        libc::SYS_bpf,
        libc::SYS_finit_module,
        libc::SYS_clone3,
    ];

    // x86_64-specific syscalls (absent on aarch64 / riscv64).
    #[cfg(target_arch = "x86_64")]
    denied.extend_from_slice(&[
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_open_by_handle_at,
        libc::SYS_kexec_file_load,
    ]);

    (arch, denied)
}

/// Install a seccomp-BPF syscall denylist on the **calling thread**.
///
/// Idempotent per OS thread: a `thread_local!` flag ensures the filter is
/// applied at most once, making it safe for Tokio's reused `spawn_blocking`
/// workers.
///
/// On failure the error is descriptive; callers should log it and continue —
/// the VM-level sandbox isolation still applies even without seccomp.
pub fn apply_denylist() -> Result<(), String> {
    if SECCOMP_APPLIED.with(|c| c.get()) {
        return Ok(());
    }

    let (arch, denied) = denylist();

    // Build the filter: each denied syscall maps to an empty rule vector,
    // which means "unconditionally apply the match_action for this syscall".
    let rules: BTreeMap<i64, Vec<SeccompRule>> =
        denied.into_iter().map(|nr| (nr, vec![])).collect();

    // mismatch_action = Allow  (syscalls NOT in the map are allowed)
    // match_action    = Errno(EPERM)  (syscalls IN the map get EPERM)
    let bpf: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(|e| format!("SeccompFilter::new failed: {}", e))?
    .try_into()
    .map_err(|e| format!("SeccompFilter compile failed: {}", e))?;

    seccompiler::apply_filter(&bpf)
        .map_err(|e| format!("seccompiler::apply_filter failed: {}", e))?;

    SECCOMP_APPLIED.with(|c| c.set(true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_denylist_succeeds() {
        // Use a dedicated OS thread to keep seccomp state isolated.
        let result = std::thread::spawn(apply_denylist)
            .join()
            .expect("thread panicked");
        assert!(result.is_ok(), "apply_denylist failed: {:?}", result);
    }

    #[test]
    fn test_apply_denylist_idempotent() {
        let result = std::thread::spawn(|| -> Result<(), String> {
            apply_denylist()?;
            apply_denylist()
        })
        .join()
        .expect("thread panicked");
        assert!(result.is_ok(), "second apply failed: {:?}", result);
    }

    /// After installing the denylist, `fork()` must return -1 with `EPERM`.
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


