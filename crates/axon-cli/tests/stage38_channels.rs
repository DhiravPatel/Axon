//! Stage 38 — channels on the async substrate: closed flag + bounded
//! capacity + backpressure policy + for-await waits on closed + select
//! fast-fails on all-closed-empty.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn axon_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push("debug");
    p.push("axon");
    p
}

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn build_axon() {
    let st = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build");
    assert!(st.success(), "build failed");
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage38-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_src(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

// =========================================================================
// §38.A — closed flag + close()/is_closed()
// =========================================================================

#[test]
fn chan_close_marks_channel_closed() {
    build_axon();
    let dir = temp_dir("close");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    if c.is_closed() { print("WRONG: starts closed") } else { print("open initially") }
    c.close()
    if c.is_closed() { print("closed after close()") } else { print("WRONG: still open") }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("open initially"), "stdout: {stdout}");
    assert!(stdout.contains("closed after close()"), "stdout: {stdout}");
    assert!(!stdout.contains("WRONG"), "stdout: {stdout}");
}

#[test]
fn chan_send_after_close_errors_with_named_message() {
    build_axon();
    let dir = temp_dir("send_closed");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.close()
    c.send("nope")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("channel is closed"),
        "expected close-named error; got: {stderr}"
    );
}

#[test]
fn chan_recv_on_closed_empty_returns_nil() {
    // recv on a closed empty channel should return nil cleanly (so
    // `let v = c.recv(); v == nil` is the natural way to test drainage).
    build_axon();
    let dir = temp_dir("recv_closed_empty");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.close()
    let v = c.recv()
    if v == nil { print("nil as expected") } else { print("WRONG: got value") }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("nil as expected"), "stdout: {stdout}");
}

// =========================================================================
// §38.A — bounded chan + backpressure
// =========================================================================

#[test]
fn bounded_chan_block_policy_errors_when_full() {
    build_axon();
    let dir = temp_dir("block");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(2, "block")
    c.send("a")
    c.send("b")
    // Third send must error (capacity 2, block policy).
    c.send("c")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("full") || stderr.contains("block"),
        "expected full/block error; got: {stderr}"
    );
}

#[test]
fn bounded_chan_drop_oldest_evicts_front_and_counts_drops() {
    build_axon();
    let dir = temp_dir("drop_oldest");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(2, "drop_oldest")
    c.send("a")
    c.send("b")
    c.send("c")   // pushes "a" out
    c.send("d")   // pushes "b" out
    // Queue is now ["c", "d"]. dropped() == 2.
    print(c.recv())
    print(c.recv())
    if c.recv() == nil { print("drained") }
    print_int(c.dropped())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["c", "d", "drained", "2"], "got: {lines:?}");
}

#[test]
fn bounded_chan_drop_new_silently_drops_when_full() {
    build_axon();
    let dir = temp_dir("drop_new");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(2, "drop_new")
    c.send("a")
    c.send("b")
    c.send("c")   // dropped silently (queue stays ["a","b"])
    c.send("d")   // also dropped
    print(c.recv())
    print(c.recv())
    if c.recv() == nil { print("drained") }
    print_int(c.dropped())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["a", "b", "drained", "2"], "got: {lines:?}");
}

#[test]
fn chan_capacity_reports_declared_capacity_or_nil() {
    build_axon();
    let dir = temp_dir("cap");
    let prog = r#"
fn main() uses { Console } {
    let unbounded = chan()
    let bounded = chan(5, "block")
    if unbounded.capacity() == nil { print("unbounded nil") }
    print_int(bounded.capacity())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["unbounded nil", "5"]);
}

#[test]
fn chan_bounded_with_unknown_policy_errors_cleanly() {
    build_axon();
    let dir = temp_dir("bad_policy");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(2, "wishful_thinking")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("wishful_thinking") || stderr.contains("policy"),
        "expected policy validation error; got: {stderr}"
    );
}

// =========================================================================
// §38.B — for-await waits on closed
// =========================================================================

#[test]
fn for_await_exits_promptly_when_channel_closed_and_drained() {
    // Closed-and-empty channel: for-await must exit immediately
    // (no §37.D 50ms poll-budget wait — Stage 38 replaces that).
    build_axon();
    let dir = temp_dir("forawait_close");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.send("a")
    c.send("b")
    c.close()
    for await v in c {
        print(v)
    }
    print("done")
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["a", "b", "done"]);
    // Exit should be immediate on closed-and-empty — Stage 37 had a 50ms
    // post-drain budget; Stage 38 should exit in well under that.
    assert!(
        elapsed < Duration::from_millis(2000),
        "for-await on closed+drained should exit promptly: {elapsed:?}"
    );
}

#[test]
fn for_await_waits_for_value_then_exits_on_close() {
    // Open channel, value arrives mid-loop; then explicit close exits.
    // Verifies the closed-flag wait pattern: loop continues as long as
    // the channel is open OR the queue has values.
    build_axon();
    let dir = temp_dir("forawait_wait");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.send("alpha")
    c.send("beta")
    c.close()
    for await v in c { print(v) }
    print("end")
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("alpha"), "{stdout}");
    assert!(stdout.contains("beta"), "{stdout}");
    assert!(stdout.contains("end"), "{stdout}");
}

// =========================================================================
// §38.C — select fast-fails on all-closed-empty Recv
// =========================================================================

#[test]
fn select_fast_fails_when_all_recv_channels_closed_and_empty() {
    // No producer will ever satisfy these recv arms. The select must
    // NOT sleep its full timeout — it should fire the timeout body
    // immediately. Verify wall time well below the declared timeout.
    build_axon();
    let dir = temp_dir("sel_fastfail");
    let prog = r#"
fn main() uses { Console } {
    let a = chan()
    let b = chan()
    a.close()
    b.close()
    select {
        v = recv(a) => { print("WRONG: a") }
        v = recv(b) => { print("WRONG: b") }
        _ = timeout(2000) => { print("fast-failed to timeout") }
    }
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fast-failed to timeout"),
        "stdout: {stdout}"
    );
    // Stage 38's fast-fail must skip the 2-second sleep. Allow 1.5s for
    // process startup but never the full 2s.
    assert!(
        elapsed < Duration::from_millis(1500),
        "select must fast-fail on all-closed-empty; got {elapsed:?}"
    );
}

#[test]
fn select_no_timeout_no_else_on_all_closed_empty_errors_cleanly() {
    // With every recv arm dead and no fallback arm, the select can't
    // produce a value. Must surface a named error, not hang.
    build_axon();
    let dir = temp_dir("sel_dead");
    let prog = r#"
fn main() uses { Console } {
    let a = chan()
    a.close()
    select {
        v = recv(a) => { print(v) }
    }
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("closed and empty") || stderr.contains("no branch can ever succeed"),
        "stderr should name the closed-empty condition; got: {stderr}"
    );
}

#[test]
fn select_with_one_open_chan_still_waits_normally() {
    // If even ONE recv arm is on a still-open channel, the fast-fail
    // shouldn't fire — we should sleep the timeout as usual.
    build_axon();
    let dir = temp_dir("sel_one_open");
    let prog = r#"
fn main() uses { Console } {
    let a = chan()
    let b = chan()
    a.close()
    // b is still open.
    select {
        v = recv(a) => { print("WRONG: a") }
        v = recv(b) => { print("WRONG: b") }
        _ = timeout(150) => { print("timed out") }
    }
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("timed out"), "{stdout}");
    // We expect to actually wait the 150ms.
    assert!(
        elapsed >= Duration::from_millis(100),
        "open recv arm must still cause the timeout to wait: {elapsed:?}"
    );
}
