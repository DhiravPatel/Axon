//! `axon-sandbox` — resource limits + subprocess sandbox.
//!
//! v0 surface:
//!
//!   * [`Limits`] — typed resource budget (`cpu_seconds`, `memory_mb`,
//!     `max_open_files`, `wall_seconds`). Serializable so policies live in
//!     `axon.toml` or a JSON document.
//!   * [`run_sandboxed`] — spawn a subprocess with `setrlimit` applied on
//!     Unix before `execve`. Returns a [`SandboxResult`] with the actual
//!     exit code, stdout, stderr, and whether a limit was breached.
//!   * [`measure_wall_only`] — pure-Rust no-rlimit fallback that just
//!     runs the command with a `wait_timeout` and records elapsed time.
//!     Used on Windows and as a deterministic test surface.
//!
//! Why subprocess rather than in-process limits? Because the runtime needs
//! to drop unsafe operations behind a real kernel boundary. The runtime's
//! own budget machinery (Stage 7) is for *Axon-level* budgets like tokens
//! and dollars; OS-level limits stop a runaway tool from eating the host.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

mod errors;
pub mod platform;

pub use errors::SandboxError;
pub use platform::{PlatformProfile, PlatformSandbox};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// CPU seconds (RLIMIT_CPU). 0 means unbounded.
    #[serde(default)]
    pub cpu_seconds: u64,
    /// Address-space cap in megabytes (RLIMIT_AS). 0 means unbounded.
    #[serde(default)]
    pub memory_mb: u64,
    /// File-descriptor cap (RLIMIT_NOFILE). 0 means unbounded.
    #[serde(default)]
    pub max_open_files: u64,
    /// Wall-clock timeout in seconds enforced by the parent process. 0
    /// means no timeout (the rlimit/RLIMIT_CPU pair is the only stop).
    #[serde(default)]
    pub wall_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        // Conservative defaults: a small sandbox should never DoS the host.
        Self {
            cpu_seconds: 10,
            memory_mb: 256,
            max_open_files: 64,
            wall_seconds: 15,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub wall_ms: u64,
    /// True if the wall-clock timeout fired and we killed the child.
    pub wall_timeout: bool,
    /// True if `exit_code` is `None` (signal/crash) — usually means a
    /// resource limit tripped on Unix (SIGXCPU, SIGSEGV from out-of-memory).
    pub limit_breached: bool,
}

/// Spawn `cmd` with `limits` applied, capturing stdout/stderr.
///
/// On Unix, `setrlimit` is applied via a `pre_exec` hook so the limits
/// take effect *before* the child's `execve`. The parent enforces
/// `wall_seconds` via polling + `kill` if the child doesn't exit in time.
///
/// On non-Unix platforms, rlimit values are accepted but only the wall
/// timeout is enforced. This is a documented v0 limit; full Windows Job
/// Object integration lands in §41 deploy.
pub fn run_sandboxed(cmd: &mut Command, limits: &Limits) -> Result<SandboxResult, SandboxError> {
    #[cfg(unix)]
    apply_rlimits(cmd, limits);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| SandboxError::Spawn(e.to_string()))?;
    let mut wall_timeout = false;
    let wall = if limits.wall_seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(limits.wall_seconds))
    };

    // Poll for exit with a 10ms grain. Cheap; no async runtime needed.
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if let Some(w) = wall {
                    if start.elapsed() >= w {
                        wall_timeout = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break std::process::ExitStatus::default();
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(SandboxError::Wait(e.to_string())),
        }
    };

    let stdout = match child.stdout.take() {
        Some(mut s) => read_to_string(&mut s),
        None => String::new(),
    };
    let stderr = match child.stderr.take() {
        Some(mut s) => read_to_string(&mut s),
        None => String::new(),
    };

    let exit_code = exit_status.code();
    let limit_breached = exit_code.is_none();
    Ok(SandboxResult {
        exit_code,
        stdout,
        stderr,
        wall_ms: start.elapsed().as_millis() as u64,
        wall_timeout,
        limit_breached,
    })
}

/// Minimal-overhead runner — no rlimits, only the wall-clock timeout. Used
/// by tests that don't depend on `setrlimit` behaviour and on platforms
/// without full sandbox support.
pub fn measure_wall_only(cmd: &mut Command, wall_seconds: u64) -> Result<SandboxResult, SandboxError> {
    let limits = Limits {
        cpu_seconds: 0,
        memory_mb: 0,
        max_open_files: 0,
        wall_seconds,
    };
    run_sandboxed(cmd, &limits)
}

fn read_to_string<R: std::io::Read>(r: &mut R) -> String {
    let mut buf = String::new();
    let _ = r.read_to_string(&mut buf);
    buf
}

#[cfg(unix)]
fn apply_rlimits(cmd: &mut Command, limits: &Limits) {
    use std::os::unix::process::CommandExt;
    let cpu = limits.cpu_seconds;
    let mem = limits.memory_mb;
    let nfd = limits.max_open_files;
    unsafe {
        cmd.pre_exec(move || {
            // Use `set_rlimit` with the resource kind constants from libc.
            if cpu > 0 {
                set_rlimit(libc::RLIMIT_CPU, cpu);
            }
            if mem > 0 {
                set_rlimit(libc::RLIMIT_AS, mem.saturating_mul(1024 * 1024));
            }
            if nfd > 0 {
                set_rlimit(libc::RLIMIT_NOFILE, nfd);
            }
            Ok(())
        });
    }
}

#[cfg(unix)]
#[cfg(target_os = "linux")]
type RlimitResource = libc::__rlimit_resource_t;
#[cfg(unix)]
#[cfg(not(target_os = "linux"))]
type RlimitResource = libc::c_int;

#[cfg(unix)]
fn set_rlimit(resource: RlimitResource, limit: u64) {
    let rl = libc::rlimit {
        rlim_cur: limit as libc::rlim_t,
        rlim_max: limit as libc::rlim_t,
    };
    unsafe {
        libc::setrlimit(resource, &rl);
    }
}

/// Resolve a path relative to the workspace root — used by tests and CLI
/// scaffolding to find binaries that live alongside `target/debug`.
pub fn workspace_relative(rel: impl Into<PathBuf>) -> PathBuf {
    let rel = rel.into();
    if rel.is_absolute() {
        return rel;
    }
    if let Ok(d) = std::env::var("CARGO_MANIFEST_DIR") {
        let mut p = PathBuf::from(d);
        p.pop();
        p.pop();
        p.push(rel);
        return p;
    }
    rel
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn echo_command_runs_to_completion() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo hello"]);
            c
        } else {
            let mut c = Command::new("/bin/sh");
            c.args(["-c", "echo hello"]);
            c
        };
        let r = run_sandboxed(&mut cmd, &Limits::default()).unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout.trim() == "hello");
        assert!(!r.wall_timeout);
        assert!(!r.limit_breached);
    }

    #[test]
    fn nonzero_exit_propagates() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit 7"]);
            c
        } else {
            let mut c = Command::new("/bin/sh");
            c.args(["-c", "exit 7"]);
            c
        };
        let r = run_sandboxed(&mut cmd, &Limits::default()).unwrap();
        assert_eq!(r.exit_code, Some(7));
        assert!(!r.limit_breached);
    }

    #[test]
    #[cfg(unix)]
    fn wall_timeout_kills_runaway_child() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30"]);
        let limits = Limits {
            cpu_seconds: 0,
            memory_mb: 0,
            max_open_files: 0,
            wall_seconds: 1,
        };
        let r = run_sandboxed(&mut cmd, &limits).unwrap();
        assert!(r.wall_timeout, "expected wall timeout");
        assert!(r.wall_ms < 5_000, "should have stopped well under 5s");
    }

    #[test]
    #[cfg(unix)]
    fn cpu_rlimit_kills_busy_loop() {
        // A `yes`-style busy loop hits 1s of CPU time fast.
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "while :; do :; done"]);
        let limits = Limits {
            cpu_seconds: 1,
            memory_mb: 0,
            max_open_files: 0,
            wall_seconds: 10,
        };
        let r = run_sandboxed(&mut cmd, &limits).unwrap();
        // RLIMIT_CPU sends SIGXCPU → exit_code is None.
        assert!(
            r.limit_breached || r.wall_timeout,
            "expected limit breach: {r:?}"
        );
    }

    #[test]
    fn limits_round_trip_through_json() {
        let l = Limits {
            cpu_seconds: 5,
            memory_mb: 128,
            max_open_files: 32,
            wall_seconds: 10,
        };
        let bytes = serde_json::to_vec(&l).unwrap();
        let back: Limits = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, l);
    }
}
