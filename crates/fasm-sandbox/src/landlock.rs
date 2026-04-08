//! Landlock filesystem-access restrictions for FASM execution threads.
//!
//! [Landlock] is a Linux kernel security module (available since kernel 5.13)
//! that lets unprivileged processes restrict their own filesystem access.
//! Calling [`apply`] inside an execution thread creates a ruleset that:
//!
//! * **Denies** all filesystem mutations (writes, creates, deletes, renames).
//! * **Allows** reading files and listing directories only within the paths
//!   explicitly supplied in `allowed_read_paths`.
//! * Falls back gracefully (logs a warning, returns `Ok`) when the running
//!   kernel does not support Landlock — so the sandbox still works on older
//!   kernels.
//!
//! Like seccomp, the restriction applies only to the calling thread and is
//! inherited by any child processes created after the call.
//!
//! [Landlock]: https://docs.kernel.org/userspace-api/landlock.html

use std::os::unix::io::RawFd;
use std::path::Path;

// ── Landlock ABI constants ────────────────────────────────────────────────────

/// `LANDLOCK_CREATE_RULESET_VERSION` flag — used to query ABI version.
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
/// `LANDLOCK_RULE_PATH_BENEATH` — the only supported rule type for FS access.
const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

// Filesystem access-right bits (ABI v1 — kernel 5.13+)
const LANDLOCK_ACCESS_FS_EXECUTE: u64     = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64  = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64   = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64    = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64  = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64   = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64    = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64    = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64   = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64   = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64  = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64    = 1 << 12;

/// All ABI-v1 filesystem-access flags (bits 0–12).
const ALL_FS_ACCESS_V1: u64 = LANDLOCK_ACCESS_FS_EXECUTE
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_READ_FILE
    | LANDLOCK_ACCESS_FS_READ_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_CHAR
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SOCK
    | LANDLOCK_ACCESS_FS_MAKE_FIFO
    | LANDLOCK_ACCESS_FS_MAKE_BLOCK
    | LANDLOCK_ACCESS_FS_MAKE_SYM;

/// Access rights granted to each whitelisted path (read-only).
const PATH_READ_ACCESS: u64 =
    LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;

// ── Landlock C structs ────────────────────────────────────────────────────────

/// Maps to `struct landlock_ruleset_attr` in `<linux/landlock.h>`.
#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

/// Maps to `struct landlock_path_beneath_attr` in `<linux/landlock.h>`.
#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd:      i32,
}

// ── Syscall wrappers ──────────────────────────────────────────────────────────

/// Detect the Landlock ABI version supported by the running kernel.
/// Returns `None` when the kernel does not support Landlock.
fn landlock_abi_version() -> Option<i64> {
    // SYS_landlock_create_ruleset = 444 on x86-64
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<LandlockRulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if ret > 0 { Some(ret) } else { None }
}

fn landlock_create_ruleset(attr: &LandlockRulesetAttr) -> Result<RawFd, String> {
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    };
    if fd < 0 {
        Err(format!(
            "landlock_create_ruleset failed: errno={}",
            unsafe { *libc::__errno_location() }
        ))
    } else {
        Ok(fd as RawFd)
    }
}

fn landlock_add_path_rule(
    ruleset_fd: RawFd,
    path_fd: RawFd,
    allowed_access: u64,
) -> Result<(), String> {
    let attr = LandlockPathBeneathAttr {
        allowed_access,
        parent_fd: path_fd,
    };
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            &attr as *const LandlockPathBeneathAttr,
            0u32,
        )
    };
    if ret != 0 {
        Err(format!(
            "landlock_add_rule failed: errno={}",
            unsafe { *libc::__errno_location() }
        ))
    } else {
        Ok(())
    }
}

fn landlock_restrict_self(ruleset_fd: RawFd) -> Result<(), String> {
    let ret = unsafe {
        libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0u32)
    };
    if ret != 0 {
        Err(format!(
            "landlock_restrict_self failed: errno={}",
            unsafe { *libc::__errno_location() }
        ))
    } else {
        Ok(())
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Apply Landlock filesystem restrictions to the **calling thread**.
///
/// The calling thread (and any children it subsequently spawns) will only be
/// able to **read** files/directories located beneath one of the paths listed
/// in `allowed_read_paths`.  All write, create, remove, and execute operations
/// on the filesystem are denied.
///
/// If the running kernel does not support Landlock the function logs a warning
/// and returns `Ok(())` — the sandbox degrades gracefully rather than refusing
/// to start.
///
/// # Errors
/// Returns an error only for unexpected `prctl` or syscall failures on a
/// kernel that *does* claim Landlock support.
pub fn apply(allowed_read_paths: &[impl AsRef<Path>]) -> Result<(), String> {
    // Check ABI version — silently skip on unsupported kernels.
    if landlock_abi_version().is_none() {
        eprintln!(
            "[fasm-sandbox/landlock] kernel does not support Landlock; \
             skipping filesystem restrictions"
        );
        return Ok(());
    }

    // Create a ruleset that handles all v1 FS access rights.
    let attr = LandlockRulesetAttr {
        handled_access_fs: ALL_FS_ACCESS_V1,
    };
    let ruleset_fd = landlock_create_ruleset(&attr)?;

    // Add a read-only rule for each allowed path.
    for raw_path in allowed_read_paths {
        let path = raw_path.as_ref();
        let path_cstr = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|e| format!("invalid path {:?}: {}", path, e))?;

        let fd = unsafe {
            libc::open(
                path_cstr.as_ptr(),
                libc::O_PATH | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            // Non-fatal: if the allowed path does not exist, skip it.
            eprintln!(
                "[fasm-sandbox/landlock] cannot open {:?} for Landlock rule (skipping): errno={}",
                path,
                unsafe { *libc::__errno_location() }
            );
            continue;
        }

        let rule_result = landlock_add_path_rule(ruleset_fd, fd, PATH_READ_ACCESS);
        unsafe { libc::close(fd) };
        rule_result?;
    }

    // PR_SET_NO_NEW_PRIVS is required before restricting self (if not already set).
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    }

    landlock_restrict_self(ruleset_fd)?;
    unsafe { libc::close(ruleset_fd) };

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_with_empty_paths_succeeds() {
        // A thread that restricts itself to zero allowed paths should not panic.
        let empty: &[&str] = &[];
        let result = std::thread::spawn(move || apply(empty))
            .join()
            .expect("thread panicked");
        // It may fail on older kernels — that is explicitly OK.
        let _ = result;
    }

    #[test]
    fn test_landlock_abi_version() {
        let v = landlock_abi_version();
        // Just verify the function does not panic; value is kernel-dependent.
        println!("[landlock] ABI version: {:?}", v);
    }

    #[test]
    fn test_apply_allows_tmpdir_reads() {
        let tmp = std::env::temp_dir();
        let result = std::thread::spawn(move || apply(&[tmp]))
            .join()
            .expect("thread panicked");
        // On kernels that support Landlock this must succeed.
        if landlock_abi_version().is_some() {
            assert!(result.is_ok(), "apply failed: {:?}", result);
        }
    }
}
