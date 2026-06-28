//! Stage 42 — optional/nullable ergonomics: `.unwrap()`, `.expect(msg)`,
//! `.unwrap_or(default)`, `.is_nil()`, `.is_some()` on any `T?` value.

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
    p.push(format!("axon-stage42-{name}-{}-{ts}", std::process::id()));
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

fn lines(out: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn unwrap_is_some_is_nil_unwrap_or() {
    build_axon();
    let dir = temp_dir("nullable");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let xs = [10, 20, 30]
    print("{xs.last().unwrap()} {xs.last().is_some()}")
    let empty: List<Int> = []
    print("{empty.first().is_nil()} {empty.first().unwrap_or(0 - 1)}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["30 true", "true -1"], "got: {:?}", lines(&out));
}

#[test]
fn expect_on_nil_errors_with_message() {
    build_axon();
    let dir = temp_dir("expect");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let e: List<Int> = []
    let v = e.first().expect("was empty")
    print("{v}")
}
"#,
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("expect: was empty"), "stderr: {err}");
}

#[test]
fn pairs_with_checked_arithmetic() {
    build_axon();
    let dir = temp_dir("checked");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let safe = 100.checked_mul(3).expect("overflow")
    print("{safe}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(lines(&out), ["300"], "got: {:?}", lines(&out));
}
