# fasm-sandbox

Isolated execution context for FASM programs.

Each `Sandbox` wraps an `fasm_vm::Executor` with its own identity, an optional
[`ClockController`] for instruction-rate throttling, and a syscall table the
host can extend at runtime.  On Linux, two additional kernel-level protection
layers can be enabled: **seccomp-BPF** and **Landlock**.

---

## Isolation Layers

FASM runs untrusted programs through three independent defence layers that
reinforce each other:

```
┌──────────────────────────────────────────────────────────────────┐
│  Layer 3 — Landlock (Linux ≥ 5.13)                               │
│  Read-only access restricted to explicitly allowed paths.         │
│  Writes, creates, removes, and executions are denied globally.    │
├──────────────────────────────────────────────────────────────────┤
│  Layer 2 — Seccomp-BPF (Linux)                                    │
│  Per-thread syscall denylist: fork/exec/socket/ptrace/kexec/…    │
│  blocked with EPERM.  Applied once per OS thread.                 │
├──────────────────────────────────────────────────────────────────┤
│  Layer 1 — VM isolation (all platforms)                           │
│  Typed memory model, fixed call-stack depth (512 frames),         │
│  explicit syscall table — every host capability must be mounted   │
│  before a program can reach it.                                   │
└──────────────────────────────────────────────────────────────────┘
```

### Layer 1 — VM isolation (all platforms)

The FASM virtual machine enforces isolation by design:

| Guarantee | Mechanism |
|---|---|
| No un-granted capability | Syscall IDs not mounted on the sandbox return `BadSyscall` fault |
| Bounded stack depth | `StackOverflow` fault at 512 call frames |
| Type safety | Every slot has a declared type; type mismatches raise `TypeMismatch` |
| Arithmetic safety | Division / modulo by zero raises `DivisionByZero`, not UB |
| No cross-sandbox state | Each `Sandbox` owns its own `Executor`, `Frame`, and `GlobalRegister` |

This layer is active on **all platforms** (Linux, macOS, Windows).

### Layer 2 — Seccomp-BPF (Linux only)

When `SandboxConfig::enable_seccomp = true`, each execution thread installs a
BPF filter (using the [`seccompiler`] crate — the same library used by
Firecracker) that returns `EPERM` for the following syscall categories:

| Category | Blocked syscalls |
|---|---|
| Process creation | `fork`, `vfork`, `clone`, `clone3`, `execve`, `execveat` |
| Networking | `socket`, `socketpair`, `bind`, `connect`, `listen`, `accept`, `accept4` |
| Filesystem mounting | `mount`, `umount2`, `pivot_root`, `chroot` |
| Kernel patching | `init_module`, `finit_module`, `delete_module`, `kexec_load`, `kexec_file_load` |
| Namespace escape | `unshare`, `setns` |
| Process introspection | `ptrace`, `process_vm_readv`, `process_vm_writev` |
| BPF self-escalation | `bpf` |
| Perf side-channel | `perf_event_open` |
| Handle-based open | `open_by_handle_at` |

The filter is installed on the **calling OS thread only** (no `TSYNC` flag),
so other Tokio runtime threads are unaffected.  A `thread_local!` flag prevents
double-installation when Tokio reuses `spawn_blocking` worker threads.

This layer is compiled in only on **Linux**.  On other OSes `enable_seccomp` is
accepted in `SandboxConfig` but has no effect.

### Layer 3 — Landlock filesystem restrictions (Linux ≥ 5.13)

When `SandboxConfig::enable_landlock = true`, each execution thread is
restricted to **read-only access** within the paths listed in
`landlock_allowed_read_paths`.  All write, create, remove, and execute
operations on the entire filesystem are denied.

The [`landlock`] crate automatically degrades gracefully on kernels older than
5.13 — the call succeeds silently and no restrictions are applied.  This means
Landlock-enabled sandboxes run safely on any kernel without requiring version
checks.

This layer is compiled in only on **Linux**.

---

## Platform Comparison

| Feature | Linux | macOS | Windows |
|---|---|---|---|
| VM isolation (typed memory, bounded stack, syscall table) | ✓ | ✓ | ✓ |
| Seccomp-BPF syscall denylist | ✓ | ✗ | ✗ |
| Landlock filesystem restrictions | ✓ (kernel ≥ 5.13) | ✗ | ✗ |
| Clock throttling (instruction-rate limit) | ✓ | ✓ | ✓ |
| IPC sidecar plugins | ✓ | ✓ | ✓ |

Linux provides the strongest isolation guarantee.  On macOS and Windows,
only the VM-level isolation (Layer 1) applies.

---

## Quick Start

```rust
use fasm_sandbox::{Sandbox, SandboxConfig};
use fasm_vm::Value;

// Minimal sandbox — VM isolation only
let mut sandbox = Sandbox::new(0);
sandbox.mount_syscall(100, Box::new(|_args, _globals| Ok(Value::Null)));
let result = sandbox.run(&program)?;

// Linux-hardened sandbox — all three layers
let config = SandboxConfig {
    enable_seccomp: true,
    enable_landlock: true,
    landlock_allowed_read_paths: vec!["/tmp".into()],
    clock_hz: 1_000_000,  // 1M instructions/tick (0 = unlimited)
    ..Default::default()
};
let mut sandbox = Sandbox::from_config(0, &config);
let result = sandbox.run(&program)?;
```

---

## `SandboxConfig` Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `clock_hz` | `u64` | `0` | Instruction-rate limit per tick. `0` = unlimited. |
| `plugin_discovery_dir` | `Option<PathBuf>` | `None` | Directory scanned for `*.plugin.toml` manifests on construction. |
| `enable_seccomp` | `bool` | `false` | Install seccomp-BPF denylist on execution threads (Linux only). |
| `enable_landlock` | `bool` | `false` | Restrict filesystem access via Landlock (Linux ≥ 5.13 only). |
| `landlock_allowed_read_paths` | `Vec<PathBuf>` | `[]` | Paths the execution thread may read. Has no effect without `enable_landlock`. |

---

## `Sandbox` API

```rust
// Construction
Sandbox::new(id: u64) -> Sandbox
Sandbox::from_config(id: u64, config: &SandboxConfig) -> Sandbox

// Capability mounting
sandbox.mount_syscall(id: i32, handler: SyscallFn)
sandbox.mount_sidecar(id: i32, cmd: &str, args: &[&str])
sandbox.mount_shared_sidecar(ids: &[i32], cmd: &str, args: &[&str])
sandbox.mount_sidecar_from_discovery(dir: &Path)

// Execution
sandbox.run(program: &Program) -> Result<Value, String>
sandbox.run_named(program: &Program, func: &str, args: Value) -> Result<Value, String>
sandbox.run_func(program: &Program, func: &str) -> Result<Value, String>

// Clock
sandbox.set_clock_hz(hz: u64)
```

### Error handling

Both `run` and `run_named` return `Ok(Value)` on success and `Err(String)` on
any VM-level fault (type mismatch, stack overflow, bad syscall, etc.).  The
caller is always in the host process — faults never propagate as panics.

On Linux, if seccomp or Landlock installation fails (e.g., running inside a
container that restricts `prctl`), the error is **logged to stderr** and
execution continues with VM-only isolation.  This fail-open behaviour
intentionally avoids breaking environments where kernel features are unavailable
while still applying them where they are.

---

## IPC Sidecar Plugins

Sidecar plugins route FASM syscall IDs to external processes over
stdin/stdout JSON-RPC.  This lets you add capabilities (database access,
ML inference, etc.) in any language without FFI.

```toml
# mydb.plugin.toml
name        = "mydb"
cmd         = "python"
args        = ["db_plugin.py"]
syscall_ids = [200, 201, 202]
auto_mount  = true
```

Place the file in the `plugin_discovery_dir` and it is automatically loaded on
`Sandbox::from_config`.

> **Note**: The sidecar channel itself (the stdin/stdout pipe) is a known trust
> boundary that will be hardened in a future release.  Until then, only mount
> sidecar plugins from trusted sources.

---

## Testing

```sh
# All sandbox unit and integration tests
cargo test -p fasm-sandbox

# Just the jailbreak / isolation tests (covers VM, seccomp, and Landlock)
cargo test -p fasm-sandbox --test jailbreak_tests

# Individual test groups
cargo test -p fasm-sandbox --test jailbreak_tests seccomp_tests
cargo test -p fasm-sandbox --test jailbreak_tests landlock_tests
```

The `jailbreak_tests` suite verifies that the sandbox cannot be escaped via
malicious FASM bytecode or a compromised syscall handler:

| Test group | What is verified |
|---|---|
| VM-level (5 tests) | Unmounted syscall IDs rejected; infinite recursion → `StackOverflow`; arithmetic faults contained; failures do not cross sandbox boundaries |
| `seccomp_tests` (5 tests, Linux) | `fork`, `socket`, `execve` blocked inside mounted handlers; benign handlers still work; VM boundary holds with seccomp active |
| `landlock_tests` (3 tests, Linux) | Read outside allowed path denied; read inside allowed path succeeds; VM boundary holds with Landlock active |
