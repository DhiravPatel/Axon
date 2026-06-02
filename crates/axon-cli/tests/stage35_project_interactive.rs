//! Stage 35.3 — project-mode `axon fix --interactive`.
//!
//! Real TTY-driven testing requires a PTY harness we don't have in v0.
//! The coverage we do get:
//!   * non-TTY fallback (pipe stdin → fall through to dry-run).
//!   * --apply path retains its prior behavior across files (the
//!     refactor that promoted the tuple to ProjectHunk shouldn't regress).
//!   * the dry-run output now includes tier labels per hunk (Stage 34
//!     already did this single-file; this checks the project mode too).

use std::path::PathBuf;
use std::process::{Command, Stdio};

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
    p.push(format!("axon-stage35pi-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, src).unwrap();
    p
}

#[test]
fn project_interactive_falls_back_to_dry_run_without_tty() {
    build_axon();
    let dir = temp_dir("fallback");
    write(
        &dir,
        "axon.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\"]\n",
    );
    write(
        &dir,
        "src/main.ax",
        "use helpers.{greet}\nfn main() uses { Console } { print(greet(\"x\")) }\n",
    );
    let helper_path = write(
        &dir,
        "src/helpers.ax",
        "fn greet(name: String) -> String { \"hi, \" + name }\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", "--interactive", dir.to_str().unwrap()])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .expect("axon fix --interactive");
    assert!(out.status.success(), "{:?}", out);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("falling back to dry-run"),
        "expected non-TTY fallback note: {combined}"
    );
    // File must not have been modified.
    let after = std::fs::read_to_string(&helper_path).unwrap();
    assert!(
        !after.starts_with("pub fn greet"),
        "non-TTY --interactive must not write the file: {after}"
    );
}

#[test]
fn project_apply_still_works_after_refactor() {
    // The refactor from a 3-tuple to ProjectHunk shouldn't regress the
    // --apply path. Round-trip: dirty project → --apply → check passes.
    build_axon();
    let dir = temp_dir("apply_regression");
    write(
        &dir,
        "axon.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\"]\n",
    );
    write(
        &dir,
        "src/main.ax",
        "use helpers.{greet}\nfn main() uses { Console } { print(greet(\"x\")) }\n",
    );
    let helper_path = write(
        &dir,
        "src/helpers.ax",
        "fn greet(name: String) -> String { \"hi, \" + name }\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", "--apply", dir.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .expect("axon fix --apply");
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&helper_path).unwrap();
    assert!(
        after.starts_with("pub fn greet"),
        "P0010 should have inserted `pub`: {after}"
    );
}

#[test]
fn project_dry_run_labels_each_hunk_with_tier() {
    build_axon();
    let dir = temp_dir("tier_labels_project");
    write(
        &dir,
        "axon.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\"]\n",
    );
    write(
        &dir,
        "src/main.ax",
        "use helpers.{greet}\nfn main() uses { Console } { print(greet(\"x\")) }\n",
    );
    write(
        &dir,
        "src/helpers.ax",
        "fn greet(name: String) -> String { \"hi, \" + name }\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", dir.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .expect("axon fix");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("P0010, safe"),
        "expected tier-labeled P0010 in dry-run output: {stdout}"
    );
}
