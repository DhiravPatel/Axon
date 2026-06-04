//! Stage 38 §38.6 verification-fix regression tests.
//!
//! Each test pins one finding from the §38.6 adversarial pass. If any
//! regresses we want the suite to go red loudly.

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
    p.push(format!("axon-stage38vf-{name}-{}-{ts}", std::process::id()));
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
// S38-001 — DropOldest at capacity=0 must preserve the capacity invariant.
//
// Pre-fix bug: `chan(0, "drop_oldest").send(v)` left len()=1, dropped()=1,
// and recv() returned v. The capacity contract says "queue length never
// exceeds n"; at n=0 the queue must always be empty regardless of policy.
// =========================================================================

#[test]
fn s38_001_drop_oldest_at_capacity_zero_keeps_queue_empty() {
    build_axon();
    let dir = temp_dir("s38_001_drop_oldest");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(0, "drop_oldest")
    c.send("x")
    c.send("y")
    c.send("z")
    // capacity=0 means we never hold anything.
    print_int(c.len())
    print_int(c.dropped())
    if c.recv() == nil { print("nil") } else { print("WRONG") }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(
        lines,
        ["0", "3", "nil"],
        "S38-001 regressed: cap=0 DropOldest must keep len=0; got: {lines:?}"
    );
}

#[test]
fn s38_001_drop_new_at_capacity_zero_keeps_queue_empty() {
    // The pre-fix bug was isolated to DropOldest; DropNew already handled
    // cap=0 correctly. Pin that behavior to prevent the fix from
    // accidentally regressing DropNew.
    build_axon();
    let dir = temp_dir("s38_001_drop_new");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(0, "drop_new")
    c.send("x")
    c.send("y")
    print_int(c.len())
    print_int(c.dropped())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["0", "2"]);
}

#[test]
fn s38_001_block_at_capacity_zero_errors_first_send() {
    // Block policy at cap=0 means "never accept any send" — every send
    // errors. Pin that behavior.
    build_axon();
    let dir = temp_dir("s38_001_block");
    let prog = r#"
fn main() uses { Console } {
    let c = chan(0, "block")
    c.send("nope")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("full") && stderr.contains("capacity 0"),
        "expected cap=0 block error; got: {stderr}"
    );
}

// =========================================================================
// DOC-003 — `eval_spawn` has a Stage 5.5 / Stage 39 docstring explaining
// it is NOT yet async despite the surface looking like it might be.
// This is a source-only assertion (grep the file).
// =========================================================================

#[test]
fn doc_003_eval_spawn_has_stage5_5_and_stage39_docstring() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../axon-runtime/src/eval.rs"
    ))
    .expect("read eval.rs");
    // Locate the fn eval_spawn declaration and inspect the preceding
    // ~30 lines. Look for the key honesty terms.
    let idx = src
        .find("fn eval_spawn(")
        .expect("eval_spawn must exist");
    let preceding = &src[idx.saturating_sub(2000)..idx];
    assert!(
        preceding.contains("Stage 5.5") && preceding.contains("synchronous"),
        "eval_spawn docstring must name Stage 5.5 + synchronous dispatch (DOC-003 regressed)"
    );
    assert!(
        preceding.contains("Stage 39"),
        "eval_spawn docstring must name Stage 39 as the async-spawn lift (DOC-003 regressed)"
    );
    assert!(
        preceding.contains("NOT") || preceding.contains("not yet"),
        "eval_spawn docstring must explicitly say spawn is NOT yet async"
    );
}
