//! Stage 27 — @approval (§25.6), prompt @version (§24.3),
//! `axon schema migrate` (§17.1 / §36) end-to-end.

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
    p.push(format!("axon-stage27-{name}-{pid}-{ts}"));
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

// ---------- §25.6 @approval ----------

#[test]
fn approval_open_approve_round_trip() {
    build_axon();
    let dir = temp_dir("approval_rt");
    let prog = r#"
fn main() uses { Console } {
    approval_open("r1", "wire_transfer", "args-here",
        "treasury@example.com", 60, "deny")
    let r = approval_get("r1")
    print(r.state)
    approval_approve("r1", "alice")
    print(approval_get("r1").state)
    print(approval_get("r1").actor)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "pending");
    assert_eq!(lines[1], "approved");
    assert_eq!(lines[2], "alice");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approval_deny_records_reason() {
    build_axon();
    let dir = temp_dir("approval_deny");
    let prog = r#"
fn main() uses { Console } {
    approval_open("r1", "wire_transfer", "args", "approver", 60, "deny")
    approval_deny("r1", "bob", "amount too high")
    let r = approval_get("r1")
    print(r.state)
    print(r.reason)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "denied");
    assert_eq!(lines[1], "amount too high");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approval_sweep_timeouts_applies_deny_directive() {
    build_axon();
    let dir = temp_dir("approval_timeout");
    let prog = r#"
fn main() uses { Console } {
    approval_open("r1", "deploy", "args", "ops", 1, "deny")
    // 10 seconds after the registration is well past the 1-second timeout.
    let fired = approval_sweep_timeouts(5000000000000000000, "")
    print(list_len(fired))
    print(approval_get("r1").state)
    print(approval_get("r1").reason)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "denied");
    assert_eq!(lines[2], "timed out");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn approval_sweep_with_escalate_marks_target() {
    build_axon();
    let dir = temp_dir("approval_escalate");
    let prog = r#"
fn main() uses { Console } {
    approval_open("r1", "deploy", "args", "ops", 1, "escalate")
    approval_sweep_timeouts(5000000000000000000, "manager@example.com")
    let r = approval_get("r1")
    print(r.state)
    print(r.escalated_to)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "timed_out");
    assert_eq!(lines[1], "manager@example.com");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §24.3 prompt @version ----------

#[test]
fn prompt_version_register_then_pick() {
    build_axon();
    let dir = temp_dir("pv_register");
    let prog = r#"
fn main() uses { Console } {
    prompt_version_register("support", "v1", "be terse", "first cut")
    prompt_version_register("support", "v2", "be terse and cite", "added citation")
    let first = prompt_version_pick("support", "")
    print(first.version)
    let v2 = prompt_version_pick("support", "v2")
    print(v2.body)
    print(v2.notes)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "v1", "first registered version is default");
    assert_eq!(lines[1], "be terse and cite");
    assert_eq!(lines[2], "added citation");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_version_set_default_promotes() {
    build_axon();
    let dir = temp_dir("pv_set_default");
    let prog = r#"
fn main() uses { Console } {
    prompt_version_register("triage", "v1", "old", "")
    prompt_version_register("triage", "v2", "new", "")
    prompt_version_set_default("triage", "v2")
    print(prompt_version_pick("triage", "").version)
    print(list_len(prompt_version_versions_for("triage")))
    print(list_len(prompt_version_prompts()))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "v2");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_version_pick_unknown_errors_cleanly() {
    build_axon();
    let dir = temp_dir("pv_unknown");
    let prog = r#"
fn main() uses { Console } {
    prompt_version_pick("never-registered", "")
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should fail on unknown prompt");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("never-registered"), "got: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- `axon schema migrate` ----------

#[test]
fn schema_inspect_counts_versions() {
    build_axon();
    let dir = temp_dir("schema_inspect");
    let store = dir.join("store.json");
    std::fs::write(
        &store,
        r#"{
            "alice": {"__schema": "Profile", "__version": 1, "name": "Alice"},
            "bob":   {"__schema": "Profile", "__version": 2, "name": "Bob"},
            "carol": {"__schema": "Profile", "__version": 1, "name": "Carol"}
        }"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "schema",
            "inspect",
            store.to_str().unwrap(),
            "--schema",
            "Profile",
        ])
        .output()
        .expect("axon schema inspect");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Profile v1: 2"), "stdout: {stdout}");
    assert!(stdout.contains("Profile v2: 1"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_migrate_plans_steps_for_outdated_entries() {
    build_axon();
    let dir = temp_dir("schema_plan");
    let store = dir.join("store.json");
    std::fs::write(
        &store,
        r#"{
            "u1": {"__schema": "Profile", "__version": 1, "x": 1},
            "u2": {"__schema": "Profile", "__version": 2, "x": 2},
            "u3": {"__schema": "Profile", "__version": 3, "x": 3}
        }"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "schema",
            "migrate",
            store.to_str().unwrap(),
            "--schema",
            "Profile",
            "--to",
            "3",
        ])
        .output()
        .expect("axon schema migrate");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PLAN Profile v1 -> v3"), "stdout: {stdout}");
    assert!(stdout.contains("steps=[1, 2]"), "stdout: {stdout}");
    assert!(stdout.contains("1 already at v3"), "stdout: {stdout}");
    assert!(stdout.contains("2 entries to upgrade"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_migrate_rejects_downgrade() {
    build_axon();
    let dir = temp_dir("schema_downgrade");
    let store = dir.join("store.json");
    std::fs::write(
        &store,
        r#"{"new": {"__schema": "Profile", "__version": 5}}"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "schema",
            "migrate",
            store.to_str().unwrap(),
            "--to",
            "3",
        ])
        .output()
        .expect("axon schema migrate");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("WOULD-DOWNGRADE"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("1 blocked"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_migrate_apply_requires_runtime_migrator() {
    build_axon();
    let dir = temp_dir("schema_apply");
    let store = dir.join("store.json");
    std::fs::write(
        &store,
        r#"{"u": {"__schema": "X", "__version": 1}}"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "schema",
            "migrate",
            store.to_str().unwrap(),
            "--to",
            "2",
            "--apply",
        ])
        .output()
        .expect("axon schema migrate");
    // Apply path bails out cleanly with a non-zero exit + helpful message.
    assert!(!out.status.success(), "should require a runtime migrator");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--apply requires a registered migrator"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
