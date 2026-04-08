use std::path::Path;
use fasm_bytecode::Program;
use fasm_vm::{Executor, Value};
use fasm_vm::value::FasmStruct;
use fasm_vm::executor::SyscallFn;
use crate::clock::ClockController;
use crate::plugin_manifest;

// ── Config types ──────────────────────────────────────────────────────────────

/// Configuration for constructing a pre-wired sandbox.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    /// Optional clock limit (0 = unlimited).
    pub clock_hz: u64,
    /// Directory to scan for `*.plugin.toml` manifests.
    pub plugin_discovery_dir: Option<std::path::PathBuf>,

    /// Enable seccomp-BPF syscall denylist on execution threads (Linux only).
    ///
    /// When `true`, each `spawn_blocking` worker installs a BPF filter that
    /// blocks dangerous syscalls (process creation, networking, ptrace, etc.)
    /// before running the FASM program.  Applied at most once per OS thread.
    pub enable_seccomp: bool,

    /// Enable Landlock filesystem restrictions on execution threads (Linux only).
    ///
    /// When `true`, execution threads are restricted to read-only access within
    /// the paths listed in `landlock_allowed_read_paths`.  Falls back gracefully
    /// on kernels that do not support Landlock (< 5.13).
    pub enable_landlock: bool,

    /// Filesystem paths the execution thread is allowed to read.
    ///
    /// Has no effect unless `enable_landlock` is `true`.
    pub landlock_allowed_read_paths: Vec<std::path::PathBuf>,
}

// ── Sandbox ───────────────────────────────────────────────────────────────────

/// An isolated execution context for a single FASM program.
pub struct Sandbox {
    pub id: u64,
    executor: Executor,
    pub clock: ClockController,
    enable_seccomp: bool,
    enable_landlock: bool,
    landlock_allowed_read_paths: Vec<std::path::PathBuf>,
}

impl Sandbox {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            executor: Executor::new(),
            clock: ClockController::new(),
            enable_seccomp: false,
            enable_landlock: false,
            landlock_allowed_read_paths: Vec::new(),
        }
    }

    /// Construct a sandbox from a [`SandboxConfig`].
    ///
    /// This reads the `plugin_discovery_dir` (if set) and auto-mounts all
    /// plugins whose manifest has `auto_mount = true`.
    pub fn from_config(id: u64, config: &SandboxConfig) -> Self {
        let mut sb = Self::new(id);
        if config.clock_hz > 0 {
            sb.set_clock_hz(config.clock_hz);
        }
        if let Some(ref dir) = config.plugin_discovery_dir {
            sb.mount_sidecar_from_discovery(dir);
        }
        sb.enable_seccomp = config.enable_seccomp;
        sb.enable_landlock = config.enable_landlock;
        sb.landlock_allowed_read_paths = config.landlock_allowed_read_paths.clone();
        sb
    }

    /// Set instructions-per-tick limit (0 = unlimited).
    pub fn set_clock_hz(&mut self, hz: u64) {
        self.clock.instructions_per_tick = hz;
    }

    /// Mount a custom syscall handler.
    pub fn mount_syscall(&mut self, id: i32, handler: SyscallFn) {
        self.executor.mount_syscall(id, handler);
    }

    /// Mount an IPC sidecar process to a single Syscall ID.
    pub fn mount_sidecar(&mut self, id: i32, cmd: &str, args: &[&str]) {
        self.mount_shared_sidecar(&[id], cmd, args);
    }

    /// Mount an IPC sidecar process across multiple Syscall IDs.
    pub fn mount_shared_sidecar(&mut self, ids: &[i32], cmd: &str, args: &[&str]) {
        use std::sync::{Arc, Mutex};
        let sidecar = crate::sidecar::SidecarPlugin::new(cmd, args);
        let locked = Arc::new(Mutex::new(sidecar));
        
        for &id in ids {
            let plg = locked.clone();
            self.mount_syscall(id, Box::new(move |val, _| {
                let mut p = plg.lock().unwrap();
                p.call(id, &val)
            }));
        }
    }

    /// Scan `dir` for `*.plugin.toml` manifests and auto-mount all plugins
    /// with `auto_mount = true`.
    ///
    /// Each plugin launches its sidecar process and routes the declared
    /// `syscall_ids` to it.  Missing or malformed manifests are logged and
    /// skipped.
    pub fn mount_sidecar_from_discovery(&mut self, dir: &Path) {
        let manifests = plugin_manifest::discover_auto_mount(dir);
        eprintln!("[fasm-sandbox] discovered {} auto-mount plugins in {:?}", manifests.len(), dir);

        for m in manifests {
            let arg_refs: Vec<&str> = m.args.iter().map(String::as_str).collect();
            eprintln!("[fasm-sandbox] mounting plugin '{}' syscalls={:?} cmd={:?}", m.name, m.syscall_ids, m.cmd);
            self.mount_shared_sidecar(&m.syscall_ids, &m.cmd, &arg_refs);
        }
    }

    /// Run the program to completion from `Main`.
    pub fn run(&mut self, program: &Program) -> Result<Value, String> {
        self.apply_thread_protections();
        self.executor.run(program)
    }

    /// Run the program starting from a named entry-point function.
    ///
    /// `args` is passed as the function's `$args` struct — useful for HTTP
    /// request handlers, scheduled tasks, and event handlers.
    pub fn run_named(&mut self, program: &Program, func: &str, args: Value) -> Result<Value, String> {
        self.apply_thread_protections();
        self.executor.run_named(program, func, args)
    }

    /// Convenience: run a named entry point with an empty `$args` struct.
    pub fn run_func(&mut self, program: &Program, func: &str) -> Result<Value, String> {
        self.executor.run_named(program, func, Value::Struct(FasmStruct::default()))
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Apply per-thread security protections before executing untrusted code.
    ///
    /// Both seccomp and Landlock are Linux-only and applied lazily (once per OS
    /// thread).  Errors are logged but do not abort execution — the VM-level
    /// sandbox isolation still applies.
    fn apply_thread_protections(&self) {
        #[cfg(target_os = "linux")]
        {
            if self.enable_seccomp {
                if let Err(e) = crate::seccomp::apply_denylist() {
                    eprintln!("[fasm-sandbox/seccomp] warning: {}", e);
                }
            }

            if self.enable_landlock {
                if let Err(e) = crate::landlock::apply(&self.landlock_allowed_read_paths) {
                    eprintln!("[fasm-sandbox/landlock] warning: {}", e);
                }
            }
        }
    }
}
