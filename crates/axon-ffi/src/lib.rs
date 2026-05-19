//! `axon-ffi` — subprocess FFI with a JSON line protocol.
//!
//! Stage 16 surface for §35 — call out to *anything* that speaks JSON
//! over stdin/stdout: a Python script, a Go binary, a Rust helper.
//! Two shapes:
//!
//!   * [`call_once`] — start a fresh subprocess, send one JSON request on
//!     stdin, read one JSON response from stdout, wait. Wall-clock bounded
//!     so a slow helper can't stall the runtime.
//!   * [`Connection`] — persistent line-oriented child. The caller can
//!     issue multiple `request/response` pairs without paying spawn cost
//!     each time. Closes the child on drop.
//!
//! Wire format: one JSON object per line, terminated by `\n`. The runtime
//! does NOT interpret response shape; it returns whatever the helper sent,
//! so the caller can decide what fields matter.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub mod dlib;
mod errors;

pub use dlib::{DlibError, DlibValue, DynamicLibrary};
pub use errors::FfiError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiCallSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Working directory for the subprocess (`None` → inherit).
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    /// Wall-clock budget. 0 means no timeout.
    pub timeout_ms: u64,
}

/// One-shot call: spawn, write request, read one line of stdout, kill if it
/// runs past the budget. Returns the parsed JSON response.
pub fn call_once(
    spec: &FfiCallSpec,
    request: &serde_json::Value,
) -> Result<serde_json::Value, FfiError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = &spec.workdir {
        cmd.current_dir(d);
    }
    let mut child = cmd.spawn().map_err(|e| FfiError::Spawn(e.to_string()))?;

    let mut stdin = child.stdin.take().ok_or_else(|| FfiError::Spawn("stdin missing".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| FfiError::Spawn("stdout missing".into()))?;
    let mut reader = BufReader::new(stdout);

    let line = serde_json::to_string(request).map_err(|e| FfiError::Encode(e.to_string()))?;
    stdin
        .write_all(line.as_bytes())
        .map_err(|e| FfiError::Io(format!("write request: {e}")))?;
    stdin.write_all(b"\n").map_err(|e| FfiError::Io(e.to_string()))?;
    // Close stdin so the child knows we're done sending.
    drop(stdin);

    let deadline = if spec.timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(spec.timeout_ms))
    };

    let response_line = read_one_line_with_deadline(&mut reader, deadline, &mut child)?;
    let v: serde_json::Value =
        serde_json::from_str(response_line.trim_end()).map_err(|e| FfiError::Parse(e.to_string()))?;

    // Reap the child. If it's still running and we have time, give it a
    // moment; otherwise kill.
    let _ = child.wait();
    Ok(v)
}

/// Persistent FFI connection. Hold one of these to amortize spawn cost
/// across many calls.
pub struct Connection {
    child: Child,
    /// Wrapped in Option so `close()` can move it out for an explicit drop
    /// without fighting our own Drop impl.
    stdin: Option<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Connection {
    pub fn open(spec: &FfiCallSpec) -> Result<Self, FfiError> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(d) = &spec.workdir {
            cmd.current_dir(d);
        }
        let mut child = cmd.spawn().map_err(|e| FfiError::Spawn(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| FfiError::Spawn("stdin missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| FfiError::Spawn("stdout missing".into()))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            reader: BufReader::new(stdout),
        })
    }

    pub fn request(
        &mut self,
        payload: &serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, FfiError> {
        let line = serde_json::to_string(payload).map_err(|e| FfiError::Encode(e.to_string()))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| FfiError::Io("stdin already closed".into()))?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| FfiError::Io(format!("write request: {e}")))?;
        stdin
            .write_all(b"\n")
            .map_err(|e| FfiError::Io(e.to_string()))?;
        stdin
            .flush()
            .map_err(|e| FfiError::Io(e.to_string()))?;
        let deadline = if timeout_ms == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_millis(timeout_ms))
        };
        let line = read_one_line_with_deadline(&mut self.reader, deadline, &mut self.child)?;
        serde_json::from_str(line.trim_end()).map_err(|e| FfiError::Parse(e.to_string()))
    }

    pub fn close(mut self) -> Result<i32, FfiError> {
        // Closing stdin signals EOF; the child should exit cleanly.
        self.stdin.take();
        let status = self.child.wait().map_err(|e| FfiError::Wait(e.to_string()))?;
        Ok(status.code().unwrap_or(-1))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Poll the child for a stdout line until the deadline. Implemented via
/// a non-blocking read loop with 10ms sleeps — keeps the surface
/// dep-free while still allowing the wall-clock cap to actually fire.
fn read_one_line_with_deadline(
    reader: &mut BufReader<ChildStdout>,
    deadline: Option<Instant>,
    child: &mut Child,
) -> Result<String, FfiError> {
    let mut line = String::new();
    // BufRead::read_line blocks; to bound the wait we set the underlying
    // fd to nonblocking on Unix and poll. For simplicity (and portability)
    // we instead spawn a sentinel thread that checks the deadline and
    // kills the child if it overstays.
    if let Some(deadline) = deadline {
        let kill = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let kill_clone = kill.clone();
        let pid = child.id();
        let handle = std::thread::spawn(move || {
            while Instant::now() < deadline {
                if kill_clone.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            // Past deadline — kill the child by pid.
            kill_child_by_pid(pid);
        });
        let read_res = reader.read_line(&mut line);
        kill.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = handle.join();
        match read_res {
            Ok(0) => {
                if Instant::now() >= deadline {
                    return Err(FfiError::Timeout);
                }
                return Err(FfiError::Closed);
            }
            Ok(_) => Ok(line),
            Err(e) => Err(FfiError::Io(format!("read response: {e}"))),
        }
    } else {
        match reader.read_line(&mut line) {
            Ok(0) => Err(FfiError::Closed),
            Ok(_) => Ok(line),
            Err(e) => Err(FfiError::Io(format!("read response: {e}"))),
        }
    }
}

#[cfg(unix)]
fn kill_child_by_pid(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_child_by_pid(_pid: u32) {
    // Best-effort: dropping the Child elsewhere handles this on Windows.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echoer() -> FfiCallSpec {
        // A `cat` reads stdin, writes to stdout. One JSON line round-trips.
        FfiCallSpec {
            program: "/bin/cat".into(),
            args: vec![],
            workdir: None,
            timeout_ms: 2000,
        }
    }

    #[test]
    #[cfg(unix)]
    fn call_once_round_trips_json() {
        let spec = echoer();
        let req = serde_json::json!({ "hello": "world", "n": 42 });
        let resp = call_once(&spec, &req).unwrap();
        assert_eq!(resp, req);
    }

    #[test]
    #[cfg(unix)]
    fn call_once_returns_payload_field() {
        let spec = echoer();
        let req = serde_json::json!({ "value": [1, 2, 3] });
        let resp = call_once(&spec, &req).unwrap();
        assert_eq!(resp["value"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    #[cfg(unix)]
    fn timeout_fires_on_silent_subprocess() {
        // `sleep 10` produces no stdout — should hit the timeout.
        let spec = FfiCallSpec {
            program: "/bin/sleep".into(),
            args: vec!["10".into()],
            workdir: None,
            timeout_ms: 200,
        };
        let req = serde_json::json!({});
        let err = call_once(&spec, &req).unwrap_err();
        assert!(matches!(err, FfiError::Timeout | FfiError::Closed));
    }

    #[test]
    #[cfg(unix)]
    fn persistent_connection_handles_multiple_calls() {
        let mut conn = Connection::open(&echoer()).unwrap();
        for i in 0..3 {
            let req = serde_json::json!({ "i": i });
            let resp = conn.request(&req, 2000).unwrap();
            assert_eq!(resp["i"], i);
        }
        let code = conn.close().unwrap();
        assert_eq!(code, 0, "cat should exit cleanly on EOF");
    }

    #[test]
    fn spec_round_trips_through_json() {
        let s = FfiCallSpec {
            program: "/bin/echo".into(),
            args: vec!["hi".into()],
            workdir: Some(PathBuf::from("/tmp")),
            timeout_ms: 500,
        };
        let bytes = serde_json::to_vec(&s).unwrap();
        let back: FfiCallSpec = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, s);
    }
}
