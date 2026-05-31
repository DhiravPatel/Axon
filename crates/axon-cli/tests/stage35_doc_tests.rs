//! Stage 35.5 — `axon test --doc` / `--doc-only`.

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
    p.push(format!("axon-stage35doc-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn project(dir: &std::path::Path, main_src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("axon.toml"),
        "[package]\nname = \"d\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.ax"), main_src).unwrap();
}

#[test]
fn doc_runs_axon_fences_extracted_from_triple_slash_comments() {
    build_axon();
    let dir = temp_dir("basic");
    project(
        &dir,
        "/// ```axon\n\
         /// assert_eq(add(1, 2), 3)\n\
         /// ```\n\
         pub fn add(a: Int, b: Int) -> Int { a + b }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("doc(main.add)"), "{stdout}");
    assert!(
        stdout.contains("fence(s) extracted"),
        "expected fence summary: {stdout}"
    );
}

#[test]
fn doc_only_drops_user_tests_and_keeps_synthesized_ones() {
    build_axon();
    let dir = temp_dir("only");
    project(
        &dir,
        "/// ```axon\n\
         /// assert_eq(add(1, 2), 3)\n\
         /// ```\n\
         pub fn add(a: Int, b: Int) -> Int { a + b }\n\
         fn main() uses { Console } {}\n\
         test \"user test\" { assert_eq(add(2, 2), 4) }\n",
    );
    let normal = Command::new(axon_bin())
        .args(["test", dir.to_str().unwrap()])
        .output()
        .expect("axon test");
    let normal_stdout = String::from_utf8_lossy(&normal.stdout);
    assert!(
        normal_stdout.contains("user test"),
        "normal run should include user test: {normal_stdout}"
    );

    let only = Command::new(axon_bin())
        .args(["test", "--doc-only", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc-only");
    let only_stdout = String::from_utf8_lossy(&only.stdout);
    assert!(
        only_stdout.contains("doc(main.add)"),
        "doc-only should include doc test: {only_stdout}"
    );
    assert!(
        !only_stdout.contains("user test"),
        "doc-only must drop user tests: {only_stdout}"
    );
}

#[test]
fn doc_ignore_flag_skips_the_block() {
    build_axon();
    let dir = temp_dir("ignore");
    project(
        &dir,
        "/// ```axon,ignore\n\
         /// assert(false)\n\
         /// ```\n\
         pub fn always_returns_one() -> Int { 1 }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The `assert(false)` would fail if it ran. Success ⇒ it didn't run.
    assert!(
        !stdout.contains("FAIL"),
        "ignored fence must not run: {stdout}"
    );
    // The fence is still counted as extracted (so a future --doc-strict
    // mode could fail on ignore counts), but no test row appears.
    assert!(stdout.contains("fence(s) extracted"), "{stdout}");
}

#[test]
fn doc_no_run_block_typechecks_without_executing() {
    // `no_run`: wrapped in `if false { ... }` so it parses + typechecks
    // (because the surrounding test block is real code) but never runs.
    build_axon();
    let dir = temp_dir("no_run");
    project(
        &dir,
        "/// ```axon,no_run\n\
         /// // would panic if executed:\n\
         /// assert(false)\n\
         /// ```\n\
         pub fn foo() -> Int { 0 }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    assert!(out.status.success(), "no_run should not fail: {:?}", out);
}

#[test]
fn doc_parse_error_in_snippet_surfaces_with_synthetic_path() {
    build_axon();
    let dir = temp_dir("parse_err");
    project(
        &dir,
        "/// ```axon\n\
         /// let x = (\n\
         /// ```\n\
         pub fn foo() -> Int { 0 }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    assert!(!out.status.success(), "broken doc snippet should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("parse error") || stderr.contains("doc-tests"),
        "expected synthesis error: {stderr}"
    );
}

#[test]
fn doc_only_on_project_with_no_fences_reports_clearly() {
    build_axon();
    let dir = temp_dir("no_fences");
    project(
        &dir,
        "pub fn foo() -> Int { 0 }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc-only", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc-only");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no doc fences found"),
        "expected explicit no-fences message: {stdout}"
    );
}
