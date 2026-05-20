//! Stage 24 — `axon optimize <prompt.ax> --eval <suite.ax>` end-to-end.

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
    p.push(format!("axon-opt-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

#[test]
fn optimize_proposes_versioned_winner() {
    build_axon();
    let dir = temp_dir("propose");
    let prompt_path = dir.join("prompt.ax");
    let suite_path = dir.join("suite.ax");
    std::fs::write(
        &prompt_path,
        r#"
// VARIANT: tone
//   = "be terse"
//   = "be thorough"
fn handle(q: String) -> String { "system: {{tone}}; user: " }
"#,
    )
    .unwrap();
    std::fs::write(&suite_path, r#"fn evaluate() -> Bool { true }"#).unwrap();

    let out = Command::new(axon_bin())
        .args([
            "optimize",
            prompt_path.to_str().unwrap(),
            "--eval",
            suite_path.to_str().unwrap(),
            "--trials",
            "2",
        ])
        .current_dir(&dir)
        .output()
        .expect("axon optimize");
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("searched 2 variant"), "stdout={stdout}");
    // The winner file should exist.
    let v1 = dir.join("prompt.v1.ax");
    assert!(v1.exists(), "expected versioned output at {}", v1.display());
    let body = std::fs::read_to_string(&v1).unwrap();
    assert!(
        body.contains("be terse") || body.contains("be thorough"),
        "winner should have one of the variant strings substituted, got: {body}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn optimize_errors_when_no_variant_markers_present() {
    build_axon();
    let dir = temp_dir("no_markers");
    let prompt_path = dir.join("prompt.ax");
    let suite_path = dir.join("suite.ax");
    std::fs::write(&prompt_path, "fn handle(q: String) -> String { q }").unwrap();
    std::fs::write(&suite_path, "fn evaluate() -> Bool { true }").unwrap();
    let out = Command::new(axon_bin())
        .args([
            "optimize",
            prompt_path.to_str().unwrap(),
            "--eval",
            suite_path.to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .expect("axon optimize");
    assert!(!out.status.success(), "should fail with no VARIANT markers");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no `// VARIANT:"), "got: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn optimize_requires_both_input_and_eval() {
    build_axon();
    let out = Command::new(axon_bin())
        .arg("optimize")
        .output()
        .expect("axon optimize");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("usage:"), "got: {stderr}");
}
