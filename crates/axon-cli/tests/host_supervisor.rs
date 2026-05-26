//! Stage 18 — supervisor restart strategies through the binary.

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
fn one_for_one_restarts_only_failing_child() {
    build_axon();
    let dir = temp_dir("sup_one_for_one");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    super_new("svc", "one_for_one", 10, 1000000000)
    super_add_child("svc", "a")
    super_add_child("svc", "b")
    super_add_child("svc", "c")
    let d = super_on_failure("svc", "b", 0)
    print(d.kind)
    print_int(list_len(d.targets))
    print(list_get(d.targets, 0))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "restart");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "b");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_for_all_restarts_every_child() {
    build_axon();
    let dir = temp_dir("sup_one_for_all");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    super_new("svc", "one_for_all", 10, 1000000000)
    super_add_child("svc", "a")
    super_add_child("svc", "b")
    super_add_child("svc", "c")
    let d = super_on_failure("svc", "b", 0)
    print_int(list_len(d.targets))
    print(list_get(d.targets, 0))
    print(list_get(d.targets, 1))
    print(list_get(d.targets, 2))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "a");
    assert_eq!(lines[2], "b");
    assert_eq!(lines[3], "c");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rest_for_one_restarts_failing_and_successors() {
    build_axon();
    let dir = temp_dir("sup_rest_for_one");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    super_new("svc", "rest_for_one", 10, 1000000000)
    super_add_child("svc", "a")
    super_add_child("svc", "b")
    super_add_child("svc", "c")
    super_add_child("svc", "d")
    let d = super_on_failure("svc", "b", 0)
    print_int(list_len(d.targets))
    print(list_get(d.targets, 0))
    print(list_get(d.targets, 1))
    print(list_get(d.targets, 2))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "b");
    assert_eq!(lines[2], "c");
    assert_eq!(lines[3], "d");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exceeding_max_restarts_escalates() {
    build_axon();
    let dir = temp_dir("sup_escalate");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    // max_restarts=2 in a 1s window — third failure escalates.
    super_new("svc", "one_for_one", 2, 1000000000)
    super_add_child("svc", "a")
    let d1 = super_on_failure("svc", "a", 0)
    let d2 = super_on_failure("svc", "a", 100000000)
    let d3 = super_on_failure("svc", "a", 200000000)
    print(d1.kind)
    print(d2.kind)
    print(d3.kind)
    print(bool(super_escalated("svc")))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "restart");
    assert_eq!(lines[1], "restart");
    assert_eq!(lines[2], "escalate");
    assert_eq!(lines[3], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failures_outside_window_are_forgotten() {
    build_axon();
    let dir = temp_dir("sup_window");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    super_new("svc", "one_for_one", 2, 1000000000)
    super_add_child("svc", "a")
    let _ = super_on_failure("svc", "a", 0)
    let _ = super_on_failure("svc", "a", 500000000)
    // 3 seconds later: the first two failures are out of the window.
    let d3 = super_on_failure("svc", "a", 3000000000)
    print(d3.kind)
    print(bool(super_escalated("svc")))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "restart");
    assert_eq!(lines[1], "false");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_child_is_reported() {
    build_axon();
    let dir = temp_dir("sup_unknown");
    let prog = r#"
fn main() uses { Console } {
    super_reset()
    super_new("svc", "one_for_one", 5, 1000000000)
    super_add_child("svc", "a")
    let d = super_on_failure("svc", "ghost", 0)
    print(d.kind)
    print(d.reason)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "unknown");
    assert!(
        lines[1].contains("ghost"),
        "reason should mention the missing child: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
