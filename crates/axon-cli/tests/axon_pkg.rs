//! Stage 23 — `axon pkg` subcommand: list / add / remove / audit deps in
//! `axon.toml`.

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
    p.push(format!("axon-pkg-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn pkg(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    let mut a: Vec<&str> = vec!["pkg"];
    a.extend_from_slice(args);
    Command::new(axon_bin())
        .args(&a)
        .current_dir(cwd)
        .output()
        .expect("axon pkg")
}

#[test]
fn pkg_list_reports_no_deps_for_fresh_manifest() {
    build_axon();
    let dir = temp_dir("list_empty");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"
"#,
    )
    .unwrap();
    let out = pkg(&["list"], &dir);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no dependencies"), "got: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_add_creates_deps_section_then_list_sees_it() {
    build_axon();
    let dir = temp_dir("add_then_list");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"
"#,
    )
    .unwrap();
    // Create the dep directory so future audits can succeed.
    std::fs::create_dir_all(dir.join("helpers").join("src")).unwrap();

    let out = pkg(&["add", "helpers", "--path", "helpers"], &dir);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("added dep `helpers`"), "got: {stdout}");

    let list_out = pkg(&["list"], &dir);
    assert!(list_out.status.success(), "{:?}", list_out);
    let ls = String::from_utf8_lossy(&list_out.stdout);
    assert!(ls.contains("helpers"), "list missing `helpers`: {ls}");
    assert!(ls.contains("path = \"helpers\""), "list missing path: {ls}");

    // Confirm the manifest still has the original package section.
    let toml = std::fs::read_to_string(dir.join("axon.toml")).unwrap();
    assert!(toml.contains("[package]"), "package table lost: {toml}");
    assert!(toml.contains("[deps.helpers]"), "deps not written: {toml}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_add_rejects_invalid_dep_name() {
    build_axon();
    let dir = temp_dir("bad_name");
    let out = pkg(&["add", "bad name!", "--path", "x"], &dir);
    assert!(!out.status.success(), "should reject bad name");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid dep name"), "got: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_remove_drops_existing_dep() {
    build_axon();
    let dir = temp_dir("remove_existing");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"

[deps.helpers]
path = "helpers"
"#,
    )
    .unwrap();
    let out = pkg(&["remove", "helpers"], &dir);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("removed dep `helpers`"));

    let after = std::fs::read_to_string(dir.join("axon.toml")).unwrap();
    assert!(!after.contains("[deps.helpers]"), "still has dep: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_remove_errors_on_missing_dep() {
    build_axon();
    let dir = temp_dir("remove_missing");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"
"#,
    )
    .unwrap();
    let out = pkg(&["remove", "ghost"], &dir);
    assert!(!out.status.success(), "should fail on missing");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no dep named `ghost`"), "got: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_audit_reports_missing_directory() {
    build_axon();
    let dir = temp_dir("audit_missing");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"

[deps.ghost]
path = "ghost"
"#,
    )
    .unwrap();
    let out = pkg(&["audit"], &dir);
    assert!(!out.status.success(), "should fail when dep is missing");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("FAIL ghost"),
        "expected FAIL for missing dep: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_audit_succeeds_for_well_formed_dep() {
    build_axon();
    let dir = temp_dir("audit_ok");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"

[deps.helpers]
path = "helpers"
"#,
    )
    .unwrap();
    // A well-formed dep has an axon.toml *or* a src/ dir.
    std::fs::create_dir_all(dir.join("helpers").join("src")).unwrap();
    let out = pkg(&["audit"], &dir);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok   helpers"), "got: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pkg_add_idempotent_overwrite() {
    build_axon();
    let dir = temp_dir("add_overwrite");
    std::fs::write(
        dir.join("axon.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"

[deps.helpers]
path = "old/path"
"#,
    )
    .unwrap();
    let out = pkg(&["add", "helpers", "--path", "new/path"], &dir);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(dir.join("axon.toml")).unwrap();
    assert!(after.contains("new/path"), "expected new path: {after}");
    assert!(!after.contains("old/path"), "old path remained: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}
