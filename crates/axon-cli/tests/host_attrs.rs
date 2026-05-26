//! Stage 18 — function-attribute runtime semantics through the binary.

use std::path::PathBuf;
use std::process::Command;

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
    let status = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build axon");
    assert!(status.success(), "build failed");
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage18-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_in(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

#[test]
fn memoize_freezes_clock_reads_for_the_same_arg() {
    build_axon();
    let dir = temp_dir("memo_basic");
    // `time_now()` is non-deterministic — every call gives a fresh nanos.
    // With `@memoize` keyed on `x`, repeated calls with `x = 7` return the
    // FIRST clock value, proving the body short-circuited.
    let prog = r#"
@memoize
fn snapshot(x: Int) -> Int uses { Time } {
    time_now()
}

fn main() uses { Console, Time } {
    let a = snapshot(7)
    // Burn some wall-clock so a fresh read would differ.
    var i = 0
    while i < 200000 { i = i + 1 }
    let b = snapshot(7)
    let c = snapshot(7)
    print_int(if a == b { 1 } else { 0 })
    print_int(if a == c { 1 } else { 0 })
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1", "second call should have returned the cached value");
    assert_eq!(lines[1], "1", "third call should have returned the cached value");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn memoize_distinguishes_different_args() {
    build_axon();
    let dir = temp_dir("memo_distinct");
    // Different keys → different cache slots → different clock reads.
    let prog = r#"
@memoize
fn snap(x: Int) -> Int uses { Time } { time_now() }

fn main() uses { Console, Time } {
    let a1 = snap(1)
    var i = 0
    while i < 200000 { i = i + 1 }
    let b1 = snap(2)
    let a2 = snap(1)   // cached → equal to a1
    let b2 = snap(2)   // cached → equal to b1
    print_int(if a1 == a2 { 1 } else { 0 })
    print_int(if b1 == b2 { 1 } else { 0 })
    print_int(if a1 != b1 { 1 } else { 0 })
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1", "snap(1) repeated should be cached");
    assert_eq!(lines[1], "1", "snap(2) repeated should be cached");
    assert_eq!(lines[2], "1", "snap(1) and snap(2) should be different cache slots");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn retry_re_runs_on_panic_then_succeeds() {
    build_axon();
    let dir = temp_dir("retry_recovers");
    let prog = r#"
@retry(times = 3)
fn flaky(counter: List<Int>) -> Int {
    list_set(counter, 0, list_get(counter, 0) + 1)
    if list_get(counter, 0) < 2 {
        panic("transient")
    }
    99
}

fn main() uses { Console } {
    let counter = list_new(0)
    print_int(flaky(counter))
    print_int(list_get(counter, 0))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "99");
    assert_eq!(lines[1], "2", "fn ran twice (1 failure + 1 success)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn retry_exhausts_and_surfaces_the_error() {
    build_axon();
    let dir = temp_dir("retry_exhaust");
    let prog = r#"
@retry(times = 2)
fn always_fails() -> Int { panic("nope") }

fn main() -> Int { always_fails() }
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nope"), "expected `nope` in stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deadline_fires_on_slow_call() {
    build_axon();
    let dir = temp_dir("deadline");
    let prog = r#"
@deadline(ms = 1)
fn slow() -> Int {
    var i = 0
    while i < 10000000 { i = i + 1 }
    i
}

fn main() { slow() }
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("deadline"),
        "expected deadline error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn idempotent_attribute_is_inert_metadata() {
    build_axon();
    let dir = temp_dir("idempotent");
    let prog = r#"
@idempotent
fn echo(x: Int) -> Int { x }

fn main() uses { Console } { print_int(echo(42)) }
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "42"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
