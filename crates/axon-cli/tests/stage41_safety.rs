//! Stage 41 — safety & ergonomics: `require` input validation, overflow-safe
//! integer arithmetic (`checked_*` / `saturating_*`), and `todo`/`unimplemented`.

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
    p.push(format!("axon-stage41-{name}-{}-{ts}", std::process::id()));
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
fn require_passes_and_fails_with_message() {
    build_axon();
    let dir = temp_dir("require_ok");
    let ok = run_src(
        &dir,
        "fn main() uses { Console } { require(2 > 1, \"x\"); print(\"ok\") }\n",
    );
    assert!(ok.status.success(), "{:?}", ok);
    assert_eq!(lines(&ok), ["ok"]);

    let dir2 = temp_dir("require_fail");
    let bad = run_src(
        &dir2,
        "fn main() uses { Console } { require(1 > 2, \"must hold\") }\n",
    );
    assert!(!bad.status.success());
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(
        err.contains("requirement failed: must hold"),
        "stderr: {err}"
    );
}

#[test]
fn checked_arithmetic_is_nil_on_overflow() {
    build_axon();
    let dir = temp_dir("checked");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let big = 9000000000000000000
    let over = big.checked_add(big)
    print("{over ?? -1}")
    let fine = 100.checked_mul(3)
    print("{fine ?? -1}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    // overflow -> nil -> coalesces to -1; in-range -> 300.
    assert_eq!(lines(&out), ["-1", "300"], "got: {:?}", lines(&out));
}

#[test]
fn saturating_arithmetic_clamps() {
    build_axon();
    let dir = temp_dir("sat");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let big = 9000000000000000000
    print("{big.saturating_add(big)}")
    let small = 0 - 9000000000000000000
    print("{small.saturating_sub(big)}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        lines(&out),
        ["9223372036854775807", "-9223372036854775808"],
        "got: {:?}",
        lines(&out)
    );
}

#[test]
fn todo_and_unimplemented_error_clearly() {
    build_axon();
    let dir = temp_dir("todo");
    let out = run_src(
        &dir,
        "fn f() -> Int { todo(\"later\") }\nfn main() uses { Console } { print(\"{f()}\") }\n",
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not yet implemented (todo): later"), "stderr: {err}");
}
