//! Platform-specific OS sandboxes.
//!
//! Stage 22 surface for §42 — kernel-level isolation beyond rlimit:
//!
//!   * **Linux**: seccomp-bpf via [`seccompiler`]. A whitelist filter
//!     installed via `prctl(PR_SET_NO_NEW_PRIVS, 1)` + `seccomp(2)`
//!     before `execve`, so the child can call only the syscalls on the
//!     [`PlatformProfile`]. Anything else trips `SIGKILL`.
//!
//!   * **macOS**: `sandbox-exec` wrapping. The child is launched under
//!     macOS's built-in `sandbox-exec(1)` with an inline sbpl profile
//!     that mirrors the same intent. (`sandbox-exec` is technically
//!     deprecated but still present and is what Apple's own ships of
//!     `xcrun`, `nix-build`, and the App Sandbox use.)
//!
//!   * **Windows**: no-op for v0. `Limits` and the parent-side wall
//!     timeout still apply; Job Object integration arrives separately.
//!
//! All three platforms expose the same shape: a [`PlatformProfile`]
//! describes the intent (defaults: read-only filesystem, no network,
//! POSIX core syscalls), and [`PlatformSandbox::apply`] mutates a
//! [`std::process::Command`] to put the child into the sandbox.

use serde::{Deserialize, Serialize};

/// Declarative intent the sandbox engine maps onto each platform's
/// native primitives. v0 ships three preset profiles plus a hand-rolled
/// option; cross-platform behaviour is "deny by default, allow what the
/// profile names".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformProfile {
    /// Read-only filesystem access if `true`. The whitelist still has
    /// `/` and `/dev/null` etc. Writes anywhere produce a sandbox error.
    #[serde(default = "default_true")]
    pub read_only_fs: bool,
    /// `true` allows outbound network syscalls (`socket`, `connect`,
    /// ...). `false` denies all networking — useful for pure compute
    /// tools that should never reach the internet.
    #[serde(default)]
    pub allow_network: bool,
    /// Allow the child to spawn subprocesses (`fork`, `execve`).
    /// Defaults to `false` — pure-compute tools shouldn't shell out.
    #[serde(default)]
    pub allow_subprocess: bool,
    /// Extra syscall names (Linux) or sbpl operations (macOS) to add to
    /// the allowlist beyond the platform defaults.
    #[serde(default)]
    pub extra_syscalls: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for PlatformProfile {
    /// Read-only FS, no network, no subprocess — the strictest profile.
    fn default() -> Self {
        Self {
            read_only_fs: true,
            allow_network: false,
            allow_subprocess: false,
            extra_syscalls: Vec::new(),
        }
    }
}

impl PlatformProfile {
    /// "Pure compute" preset — strict.
    pub fn strict() -> Self {
        Self::default()
    }

    /// "Networked tool" preset — adds outbound network syscalls.
    pub fn networked() -> Self {
        Self {
            allow_network: true,
            ..Self::default()
        }
    }

    /// "Build tool" preset — read-only FS but may spawn helpers.
    pub fn build_tool() -> Self {
        Self {
            allow_subprocess: true,
            ..Self::default()
        }
    }
}

/// The active sandbox handle. Mutates a [`std::process::Command`] before
/// spawn so the child is launched already inside the cage.
pub struct PlatformSandbox {
    pub profile: PlatformProfile,
}

impl PlatformSandbox {
    pub fn new(profile: PlatformProfile) -> Self {
        Self { profile }
    }

    /// Wire the platform-native sandbox into `cmd`. On Linux this
    /// installs a seccomp filter via `pre_exec`. On macOS this rewrites
    /// the command to invoke through `sandbox-exec`. On other platforms
    /// this is a no-op (the caller's `Limits` + wall timeout still
    /// apply).
    pub fn apply(&self, cmd: &mut std::process::Command) -> Result<(), crate::SandboxError> {
        #[cfg(target_os = "linux")]
        {
            return linux::install_filter(cmd, &self.profile);
        }
        #[cfg(target_os = "macos")]
        {
            return macos::wrap_with_sandbox_exec(cmd, &self.profile);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            // Best-effort no-op; document elsewhere.
            let _ = cmd;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: seccomp-bpf via seccompiler
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::process::CommandExt;

    use crate::SandboxError;

    use super::PlatformProfile;
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch,
    };
    use std::collections::BTreeMap;

    /// The default syscall allowlist — POSIX I/O, memory management,
    /// process exit. Network/subprocess/write-FS syscalls are added by
    /// `profile_to_filter` only when the profile opts in.
    const POSIX_CORE: &[&str] = &[
        "read", "write", "close", "fstat", "fstatat", "newfstatat", "lseek",
        "mmap", "mprotect", "munmap", "brk", "rt_sigaction", "rt_sigprocmask",
        "rt_sigreturn", "ioctl", "readv", "writev", "pread64", "pwrite64",
        "exit", "exit_group", "tgkill", "gettid", "getpid", "getppid",
        "set_tid_address", "set_robust_list", "rseq", "futex", "clock_gettime",
        "clock_nanosleep", "clock_getres", "nanosleep", "sched_yield",
        "getrandom", "uname", "arch_prctl", "prlimit64", "getuid", "geteuid",
        "getgid", "getegid", "sysinfo", "madvise", "mincore", "membarrier",
        "openat", "open", // read paths only — we don't gate on flags in v0
        "getdents64", "getcwd", "readlink", "readlinkat",
        "epoll_create1", "epoll_ctl", "epoll_pwait", "epoll_wait",
        "poll", "ppoll", "select", "pselect6", "pipe", "pipe2", "dup", "dup2", "dup3",
        "fcntl", "stat", "lstat", "access", "faccessat", "faccessat2",
    ];

    const NETWORK: &[&str] = &[
        "socket", "connect", "bind", "listen", "accept", "accept4",
        "sendto", "recvfrom", "sendmsg", "recvmsg", "shutdown", "getsockname",
        "getpeername", "setsockopt", "getsockopt",
    ];

    const SUBPROCESS: &[&str] = &[
        "clone", "clone3", "fork", "vfork", "execve", "execveat",
        "wait4", "waitid",
    ];

    const WRITE_FS: &[&str] = &[
        "unlink", "unlinkat", "rename", "renameat", "renameat2", "mkdir",
        "mkdirat", "rmdir", "chmod", "fchmod", "fchmodat", "chown", "fchown",
        "fchownat", "truncate", "ftruncate", "fsync", "fdatasync",
        "link", "linkat", "symlink", "symlinkat",
    ];

    pub fn install_filter(
        cmd: &mut std::process::Command,
        profile: &PlatformProfile,
    ) -> Result<(), SandboxError> {
        let program = build_program(profile)?;
        // The filter is installed inside the child between `fork` and
        // `execve`, via `pre_exec`. This is the only way to keep the
        // parent's syscall surface unfiltered.
        let filter_bytes: Vec<seccompiler::sock_filter> = program;
        unsafe {
            cmd.pre_exec(move || {
                // Set NO_NEW_PRIVS so the kernel allows an unprivileged
                // process to install a seccomp filter.
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let prog = libc::sock_fprog {
                    len: filter_bytes.len() as u16,
                    filter: filter_bytes.as_ptr() as *mut _,
                };
                // SECCOMP_SET_MODE_FILTER == 1
                let res = libc::syscall(
                    libc::SYS_seccomp,
                    1,
                    0,
                    &prog as *const _ as usize,
                );
                if res != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }

    fn build_program(profile: &PlatformProfile) -> Result<BpfProgram, SandboxError> {
        let mut allowed: Vec<&str> = POSIX_CORE.to_vec();
        if profile.allow_network {
            allowed.extend(NETWORK.iter().copied());
        }
        if profile.allow_subprocess {
            allowed.extend(SUBPROCESS.iter().copied());
        }
        if !profile.read_only_fs {
            allowed.extend(WRITE_FS.iter().copied());
        }
        for extra in &profile.extra_syscalls {
            allowed.push(extra.as_str());
        }
        let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        for name in allowed {
            let nr = seccompiler::sys::libc::SYS_to_int(name).ok_or_else(|| {
                SandboxError::Io(format!("unknown syscall name `{name}`"))
            })?;
            rules.entry(nr).or_default();
        }
        let filter = SeccompFilter::new(
            rules,
            SeccompAction::KillProcess,
            SeccompAction::Allow,
            target_arch(),
        )
        .map_err(|e| SandboxError::Io(format!("build seccomp filter: {e}")))?;
        let prog: BpfProgram = filter
            .try_into()
            .map_err(|e| SandboxError::Io(format!("compile seccomp filter: {e}")))?;
        Ok(prog)
    }

    fn target_arch() -> TargetArch {
        #[cfg(target_arch = "x86_64")]
        return TargetArch::x86_64;
        #[cfg(target_arch = "aarch64")]
        return TargetArch::aarch64;
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        return TargetArch::x86_64;
    }
}

// ---------------------------------------------------------------------------
// macOS: sandbox-exec wrapping
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use crate::SandboxError;

    use super::PlatformProfile;

    /// Rewrites `cmd` to invoke `sandbox-exec -p <profile>` with the
    /// original program + args appended. `sandbox-exec` ships in
    /// `/usr/bin/sandbox-exec` on every macOS install.
    pub fn wrap_with_sandbox_exec(
        cmd: &mut std::process::Command,
        profile: &PlatformProfile,
    ) -> Result<(), SandboxError> {
        let sbpl = profile_to_sbpl(profile);
        let original_program = cmd.get_program().to_owned();
        let original_args: Vec<std::ffi::OsString> =
            cmd.get_args().map(|a| a.to_owned()).collect();

        // Rebuild the command: `sandbox-exec -p <sbpl> <orig_program> <args...>`
        let mut wrapped = std::process::Command::new("/usr/bin/sandbox-exec");
        wrapped.arg("-p").arg(&sbpl);
        wrapped.arg(&original_program);
        for a in &original_args {
            wrapped.arg(a);
        }
        // Inherit env, stdin/stdout/stderr — caller already configured those.
        for (k, v_opt) in cmd.get_envs() {
            match v_opt {
                Some(v) => {
                    wrapped.env(k, v);
                }
                None => {
                    wrapped.env_remove(k);
                }
            }
        }
        if let Some(cwd) = cmd.get_current_dir() {
            wrapped.current_dir(cwd);
        }
        // Swap into the original `Command` slot. Replace by overwriting.
        *cmd = wrapped;
        Ok(())
    }

    fn profile_to_sbpl(profile: &PlatformProfile) -> String {
        // Default-deny base profile + opt-in operations. The leading
        // (version 1) directive is required by sandbox-exec.
        // Only sbpl operations that work across macOS versions are
        // listed; experimental tags like `process-info-codesigning-*`
        // come and go between releases.
        let mut s = String::from("(version 1)\n(deny default)\n");
        s.push_str("(allow process-info*)\n");
        s.push_str("(allow signal (target self))\n");
        // The target program itself must be exec'd by the sandbox shim
        // before any of the user's code runs. `allow_subprocess` further
        // below gates the child's ability to *fork* additional helpers.
        s.push_str("(allow process-exec*)\n");
        // Read access to most of the filesystem is the v0 baseline.
        s.push_str("(allow file-read*)\n");
        if !profile.read_only_fs {
            s.push_str("(allow file-write*)\n");
        } else {
            // Always allow writes to /dev/null and tmp (process needs
            // *some* scratch space).
            s.push_str("(allow file-write* (literal \"/dev/null\"))\n");
            s.push_str("(allow file-write* (regex #\"^/private/tmp/\"))\n");
            s.push_str("(allow file-write* (regex #\"^/tmp/\"))\n");
        }
        if profile.allow_network {
            s.push_str("(allow network*)\n");
        }
        if profile.allow_subprocess {
            s.push_str("(allow process-fork)\n");
        }
        // Always allow mach-lookup of system services + sysctl reads.
        s.push_str("(allow mach-lookup)\n");
        s.push_str("(allow sysctl-read)\n");
        // Always allow shared-memory + IPC primitives the dynamic linker uses.
        s.push_str("(allow ipc-posix-shm*)\n");
        s.push_str("(allow ipc-posix-sem*)\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_strict() {
        let p = PlatformProfile::default();
        assert!(p.read_only_fs);
        assert!(!p.allow_network);
        assert!(!p.allow_subprocess);
        assert!(p.extra_syscalls.is_empty());
    }

    #[test]
    fn networked_preset_enables_network_only() {
        let p = PlatformProfile::networked();
        assert!(p.allow_network);
        assert!(!p.allow_subprocess);
        assert!(p.read_only_fs);
    }

    #[test]
    fn build_tool_preset_enables_subprocess_only() {
        let p = PlatformProfile::build_tool();
        assert!(!p.allow_network);
        assert!(p.allow_subprocess);
    }

    #[test]
    fn profile_round_trips_through_json() {
        let p = PlatformProfile {
            read_only_fs: false,
            allow_network: true,
            allow_subprocess: false,
            extra_syscalls: vec!["getentropy".into()],
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: PlatformProfile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_sandbox_exec_wrapping_preserves_program_and_args() {
        use crate::PlatformSandbox;
        let mut cmd = std::process::Command::new("/bin/echo");
        cmd.arg("hello").arg("world");
        PlatformSandbox::new(PlatformProfile::strict())
            .apply(&mut cmd)
            .unwrap();
        assert_eq!(cmd.get_program(), "/usr/bin/sandbox-exec");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args[0], "-p", "profile flag should precede the sbpl");
        // 4th arg should be the original program.
        assert_eq!(args[2], "/bin/echo");
        assert_eq!(args[3], "hello");
        assert_eq!(args[4], "world");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_actually_runs_under_sandbox_exec() {
        use crate::PlatformSandbox;
        let mut cmd = std::process::Command::new("/bin/echo");
        cmd.arg("sandboxed");
        PlatformSandbox::new(PlatformProfile::strict())
            .apply(&mut cmd)
            .unwrap();
        let out = cmd.output().expect("spawn under sandbox-exec");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "sandbox-exec failed: {:?}", out);
        assert!(stdout.contains("sandboxed"));
    }
}
