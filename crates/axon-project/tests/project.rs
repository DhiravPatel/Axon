//! Integration tests for the project loader.
//!
//! Tests write small temporary projects to disk via `tempdir`-free helpers
//! (we just use `std::env::temp_dir()` + a randomized subdir) and assert
//! the loader's behavior. Manifest parsing has its own dedicated tests
//! since they don't need disk.

use std::path::PathBuf;

use axon_project::{LoadedProject, Manifest};

fn make_temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-project-test-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, contents).expect("write file");
}

// ===========================================================================
// Manifest
// ===========================================================================

#[test]
fn manifest_defaults_when_absent() {
    let dir = make_temp_dir("no_manifest");
    let m = Manifest::from_dir(&dir).unwrap();
    assert_eq!(m.run.entry, "main");
    assert_eq!(m.run.src, "src");
    assert!(m.caps.default.is_empty());
}

#[test]
fn manifest_parses_typical_axon_toml() {
    let toml = r#"
[package]
name = "hello"
version = "0.1.0"

[run]
entry = "main"
src = "src"

[caps]
default = ["Console", "Net"]
"#;
    let m = Manifest::parse(toml).unwrap();
    assert_eq!(m.package.name, "hello");
    assert_eq!(m.package.version, "0.1.0");
    assert_eq!(m.caps.default, vec!["Console".to_string(), "Net".to_string()]);
}

#[test]
fn manifest_unknown_fields_are_accepted() {
    let toml = r#"
[package]
name = "hello"

[future_section]
key = "value"
"#;
    let m = Manifest::parse(toml).unwrap();
    assert_eq!(m.package.name, "hello");
}

// ===========================================================================
// Module loading
// ===========================================================================

#[test]
fn loads_single_file_as_one_module() {
    let dir = make_temp_dir("single");
    let f = dir.join("hello.ax");
    write(&f, "fn main() { 0 }");
    let p = LoadedProject::load(&f).expect("load");
    assert_eq!(p.modules.len(), 1);
    assert_eq!(p.merged.items.len(), 1);
    assert!(p.diagnostics.is_empty());
}

#[test]
fn loads_directory_with_manifest_and_multiple_files() {
    let dir = make_temp_dir("multi");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/a.ax"), "pub fn aa() -> Int { 1 }");
    write(&dir.join("src/b.ax"), "pub fn bb() -> Int { 2 }");
    let p = LoadedProject::load(&dir).expect("load");
    assert_eq!(p.modules.len(), 2);
    assert_eq!(p.merged.items.len(), 2);
    assert!(p.diagnostics.is_empty());
}

#[test]
fn nested_directories_become_dotted_module_paths() {
    let dir = make_temp_dir("nested");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/util/strings.ax"), "pub fn id(s: String) -> String { s }");
    let p = LoadedProject::load(&dir).expect("load");
    let strings = p
        .modules
        .iter()
        .find(|m| m.module_path == "util.strings")
        .expect("util.strings module");
    let _ = strings;
}

#[test]
fn name_collision_across_modules_is_a_diagnostic() {
    let dir = make_temp_dir("collision");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/a.ax"), "pub fn shared() -> Int { 1 }");
    write(&dir.join("src/b.ax"), "pub fn shared() -> Int { 2 }");
    let p = LoadedProject::load(&dir).expect("load");
    assert!(
        !p.diagnostics.is_empty(),
        "expected collision diagnostic, got {:#?}",
        p.diagnostics
    );
    let diag = &p.diagnostics[0];
    assert_eq!(diag.code, Some("P0001"));
    assert!(diag.message.contains("shared"));
}

#[test]
fn missing_src_directory_is_a_clear_error() {
    let dir = make_temp_dir("nosrc");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    let err = match LoadedProject::load(&dir) {
        Ok(_) => panic!("expected an error when src/ is missing"),
        Err(e) => e,
    };
    assert!(err.contains("src"), "msg = {err}");
}

#[test]
fn custom_src_dir_from_manifest_is_honored() {
    let dir = make_temp_dir("custom_src");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n[run]\nsrc = \"lib\"\n",
    );
    write(&dir.join("lib/hi.ax"), "pub fn hi() -> Int { 1 }");
    let p = LoadedProject::load(&dir).expect("load");
    assert_eq!(p.modules.len(), 1);
}
