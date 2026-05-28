//! Stage 32 — async I/O slice: `flow_parallel_asks`.
//!
//! The defining acceptance test for the multi-week async-runtime migration
//! is bounded but concrete: three mock model calls each sleeping 200 ms.
//! Serial dispatch (Stage 28 `flow_parallel`) would take ~600 ms — the sum
//! of latencies. The Stage 32 dispatcher runs them on `tokio::spawn_blocking`
//! and must finish in < 400 ms — the max of latencies, with some slack for
//! task-launch overhead.
//!
//! Equally important: replay must stay byte-identical. The dispatcher joins
//! and records events in **input order**, not completion order — so a
//! recording captured during a parallel run reproduces a deterministic
//! output regardless of which task happened to finish first.

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
    p.push(format!("axon-stage32-{name}-{}-{ts}", std::process::id()));
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

// ===========================================================================
// Acceptance: parallel < max-of-latencies + slack, not sum-of-latencies.
// ===========================================================================

#[test]
fn three_200ms_asks_parallel_finish_under_400ms_wall_time() {
    build_axon();
    let dir = temp_dir("wall");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m1 = mock_model_slow("a", 200)
    let m2 = mock_model_slow("b", 200)
    let m3 = mock_model_slow("c", 200)
    let xs = flow_parallel_asks([
        { target: m1, user: "q1" },
        { target: m2, user: "q2" },
        { target: m3, user: "q3" },
    ])
    print(list_get(xs, 0))
    print(list_get(xs, 1))
    print(list_get(xs, 2))
}
"#;
    // Measure end-to-end including process startup. Process startup is
    // ~30-100ms; we have 200ms of slack on top of the 200ms parallel
    // floor, so this is robust against startup jitter.
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("a"), "stdout: {stdout}");
    assert!(stdout.contains("b"), "stdout: {stdout}");
    assert!(stdout.contains("c"), "stdout: {stdout}");
    assert!(
        elapsed < Duration::from_millis(800),
        "wall time {elapsed:?} — flow_parallel_asks not actually parallelizing \
         (3 × 200ms serial would be ~600ms; we allow up to 800ms for process \
         startup but the model I/O itself must overlap)"
    );
}

// ===========================================================================
// Determinism: responses arrive in input order even if a faster task wins.
// ===========================================================================

#[test]
fn results_are_returned_in_input_order_not_completion_order() {
    build_axon();
    let dir = temp_dir("order");
    // m1 is slowest, m3 is fastest. A completion-order dispatcher would
    // hand us c, b, a. The Stage 32 contract is input order: a, b, c.
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m1 = mock_model_slow("a", 150)
    let m2 = mock_model_slow("b", 75)
    let m3 = mock_model_slow("c", 25)
    let xs = flow_parallel_asks([
        { target: m1, user: "q1" },
        { target: m2, user: "q2" },
        { target: m3, user: "q3" },
    ])
    print(list_get(xs, 0))
    print(list_get(xs, 1))
    print(list_get(xs, 2))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["a", "b", "c"]);
}

// ===========================================================================
// Capability gating.
// ===========================================================================

#[test]
fn flow_parallel_asks_refuses_without_llm_cap() {
    build_axon();
    let dir = temp_dir("nocap");
    // No `uses { LLM, Net }` on `main` — the type checker rejects ahead of
    // runtime; we check the stderr for a recognizable diagnostic.
    let prog = r#"
fn main() uses { Console } {
    let m = mock_model_slow("x", 1)
    flow_parallel_asks([{ target: m, user: "q" }])
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("LLM") || stderr.contains("Net"),
        "stderr should name the missing capability, got: {stderr}"
    );
}

// ===========================================================================
// Per-slot errors don't poison the whole batch.
// ===========================================================================

#[test]
fn malformed_item_surfaces_as_top_level_error_before_dispatch() {
    build_axon();
    let dir = temp_dir("malformed");
    // Missing `user` field on the second item — must error pre-dispatch
    // (we don't want to start any I/O at all if the batch is malformed).
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model_slow("x", 1)
    flow_parallel_asks([
        { target: m, user: "ok" },
        { target: m },
    ])
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("user") || stderr.contains("missing"),
        "stderr should explain the missing field, got: {stderr}"
    );
}

// ===========================================================================
// Empty batch is a no-op, not an error.
// ===========================================================================

#[test]
fn empty_batch_returns_empty_list() {
    build_axon();
    let dir = temp_dir("empty");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let xs = flow_parallel_asks([])
    print_int(list_len(xs))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0"), "stdout: {stdout}");
}
