//! Stage 13 — `flow_*` orchestration + reasoning through the binary.

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
    p.push(format!("axon-stage13-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_program(src: &str) -> std::process::Output {
    build_axon();
    let dir = temp_dir("run");
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn flow_seq_threads_value_through_named_functions() {
    let out = run_program(
        r#"
fn add_one(x: Int) -> Int { x + 1 }
fn double(x: Int) -> Int { x * 2 }

fn main() uses { Console } {
    let result = flow_seq(list_new(add_one, double, add_one), 5)
    // (5+1)*2 + 1 = 13
    print_int(result)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("13"), "expected 13, got: {stdout:?}");
}

#[test]
fn flow_parallel_fans_out_to_all_steps() {
    let out = run_program(
        r#"
fn plus_one(x: Int) -> Int { x + 1 }
fn times_two(x: Int) -> Int { x * 2 }
fn minus_five(x: Int) -> Int { x - 5 }

fn main() uses { Console } {
    let results = flow_parallel(
        list_new(plus_one, times_two, minus_five),
        10
    )
    print_int(list_len(results))
    print_int(list_get(results, 0))
    print_int(list_get(results, 1))
    print_int(list_get(results, 2))
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "3", "len should be 3");
    assert_eq!(lines[1], "11", "plus_one(10) = 11");
    assert_eq!(lines[2], "20", "times_two(10) = 20");
    assert_eq!(lines[3], "5", "minus_five(10) = 5");
}

#[test]
fn flow_refine_accepts_when_first_draft_clears_threshold() {
    let out = run_program(
        r#"
fn propose() -> Int { 100 }
fn score(d: Int) -> Int { d }
fn revise(d: Int, s: Int) -> Int { d + 1 }

fn main() uses { Console } {
    let r = flow_refine(propose, score, revise, 5, 50)
    print(r.outcome)
    print_int(r.draft)
    print_int(r.rounds)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "accepted");
    assert_eq!(lines[1], "100");
    assert_eq!(lines[2], "0", "no revisions needed");
}

#[test]
fn flow_refine_iterates_until_threshold_then_stops() {
    let out = run_program(
        r#"
// First draft scores 3; revise(d, _) returns d+1 → score grows by 1 each round.
fn propose() -> Int { 3 }
fn score(d: Int) -> Int { d }
fn revise(d: Int, s: Int) -> Int { d + 1 }

fn main() uses { Console } {
    let r = flow_refine(propose, score, revise, 10, 6)
    print(r.outcome)
    print_int(r.draft)
    print_int(r.rounds)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "accepted");
    assert_eq!(lines[1], "6", "draft = 6 reaches threshold");
    assert_eq!(lines[2], "3", "after 3 revisions");
}

#[test]
fn flow_refine_returns_best_on_max_rounds() {
    let out = run_program(
        r#"
// Score never reaches threshold; expect best-so-far back.
fn propose() -> Int { 0 }
fn score(d: Int) -> Int { d }
fn revise(d: Int, s: Int) -> Int { d + 1 }

fn main() uses { Console } {
    let r = flow_refine(propose, score, revise, 3, 100)
    print(r.outcome)
    print_int(r.draft)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "max_rounds");
    assert_eq!(lines[1], "3", "best draft after 3 rounds");
}

#[test]
fn flow_seq_propagates_runtime_error_with_step_index() {
    let out = run_program(
        r#"
fn ok(x: Int) -> Int { x }
fn boom(x: Int) -> Int { panic("step blew up") }

fn main() {
    flow_seq(list_new(ok, boom, ok), 1)
}
"#,
    );
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("flow_seq[1]"),
        "expected step index in error, got: {stderr:?}"
    );
}
