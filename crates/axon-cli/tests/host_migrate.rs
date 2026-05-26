//! Stage 18 — schema migrations through the binary.

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
    p.push(format!("axon-stage18m-{name}-{pid}-{ts}"));
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
fn no_op_when_already_at_current_version() {
    build_axon();
    let dir = temp_dir("noop");
    let prog = r#"
fn main() uses { Console } {
    schema_migrate_reset()
    schema_migrator_new("Profile", 3)
    let r = schema_migrate("Profile", "already-v3", 3)
    print(bool(r.ok))
    print(r.value)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "already-v3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn walks_chain_from_v1_to_v3() {
    build_axon();
    let dir = temp_dir("chain");
    // Each step concatenates a marker so the test can see the chain ran
    // in order: v1 → v2 → v3.
    let prog = r#"
fn upgrade_v1(old: String) -> String {
    str_join("", list_new(old, "+v2"))
}
fn upgrade_v2(old: String) -> String {
    str_join("", list_new(old, "+v3"))
}

fn main() uses { Console } {
    schema_migrate_reset()
    schema_migrator_new("Profile", 3)
    schema_add_migration("Profile", 1, upgrade_v1)
    schema_add_migration("Profile", 2, upgrade_v2)
    let r = schema_migrate("Profile", "raw", 1)
    print(bool(r.ok))
    print(r.value)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "raw+v2+v3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_step_in_chain_is_reported() {
    build_axon();
    let dir = temp_dir("missing");
    let prog = r#"
fn bump(old: String) -> String { str_join("", list_new(old, "+v3")) }

fn main() uses { Console } {
    schema_migrate_reset()
    schema_migrator_new("Profile", 3)
    // Only the v2 → v3 step registered; v1 → v2 is missing.
    schema_add_migration("Profile", 2, bump)
    let r = schema_migrate("Profile", "raw", 1)
    print(bool(r.ok))
    print(r.error)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert!(
        lines[1].contains("from = 1"),
        "expected v1 to be flagged as missing: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn downgrade_is_refused() {
    build_axon();
    let dir = temp_dir("downgrade");
    let prog = r#"
fn main() uses { Console } {
    schema_migrate_reset()
    schema_migrator_new("Profile", 2)
    // Stored value claims to be v5 but our schema is only v2.
    let r = schema_migrate("Profile", "data", 5)
    print(bool(r.ok))
    print(r.error)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert!(
        lines[1].contains("downgrade") || lines[1].contains("refusing"),
        "expected downgrade error: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn handler_error_short_circuits_chain() {
    build_axon();
    let dir = temp_dir("handler_err");
    let prog = r#"
fn good(old: String) -> String { str_join("", list_new(old, "+ok")) }
fn boom(old: String) -> String { panic("v2 step blew up") }
fn never(old: String) -> String { str_join("", list_new(old, "+never")) }

fn main() uses { Console } {
    schema_migrate_reset()
    schema_migrator_new("Profile", 4)
    schema_add_migration("Profile", 1, good)
    schema_add_migration("Profile", 2, boom)
    schema_add_migration("Profile", 3, never)
    let r = schema_migrate("Profile", "x", 1)
    print(bool(r.ok))
    print(r.error)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert!(
        lines[1].contains("v2 step blew up"),
        "expected panic message in error: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn registering_step_above_current_is_rejected() {
    build_axon();
    let dir = temp_dir("invalid_step");
    let prog = r#"
fn id(x: String) -> String { x }

fn main() {
    schema_migrate_reset()
    schema_migrator_new("Profile", 2)
    schema_add_migration("Profile", 5, id)   // panics: 5 >= 2
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("from = 5") || stderr.contains("v2 schema"),
        "expected helpful error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
