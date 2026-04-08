//! Landlock filesystem-access restrictions for FASM execution threads.
//!
//! Uses the official [`landlock`] crate to restrict the calling thread to
//! read-only access within the paths listed in `allowed_read_paths`.  All
//! write, create, delete, and execute operations on the filesystem are denied.
//!
//! The [`landlock`] crate automatically detects the kernel ABI version and
//! degrades gracefully on kernels that do not fully support Landlock (< 5.13),
//! so no explicit version check is needed here.

use std::path::Path;

use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI,
};

/// Apply Landlock filesystem restrictions to the **calling thread**.
///
/// The thread (and any children it creates afterward) will only be able to
/// **read** files and directories within `allowed_read_paths`.  All write,
/// create, remove, and execute operations are denied.
///
/// On kernels that do not support Landlock the call completes silently — the
/// `landlock` crate degrades gracefully via its `BestEffort` compatibility mode.
///
/// # Errors
/// Returns an error only for unexpected failures (e.g., invalid path strings).
pub fn apply(allowed_read_paths: &[impl AsRef<Path>]) -> Result<(), String> {
    let abi = ABI::V1;

    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| format!("Ruleset::handle_access failed: {}", e))?
        .create()
        .map_err(|e| format!("Ruleset::create failed: {}", e))?;

    let access_read = AccessFs::from_read(abi);
    for path in allowed_read_paths {
        let fd = PathFd::new(path)
            .map_err(|e| format!("PathFd::new({:?}) failed: {}", path.as_ref(), e))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, access_read))
            .map_err(|e| format!("add_rule({:?}) failed: {}", path.as_ref(), e))?;
    }

    ruleset
        .restrict_self()
        .map_err(|e| format!("restrict_self failed: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_with_empty_paths_succeeds() {
        let empty: &[&str] = &[];
        let result = std::thread::spawn(move || apply(empty))
            .join()
            .expect("thread panicked");
        assert!(result.is_ok(), "apply(empty) failed: {:?}", result);
    }

    #[test]
    fn test_apply_allows_tmpdir_reads() {
        let tmp = std::env::temp_dir();
        let result = std::thread::spawn(move || apply(&[tmp]))
            .join()
            .expect("thread panicked");
        assert!(result.is_ok(), "apply([tmp]) failed: {:?}", result);
    }
}
