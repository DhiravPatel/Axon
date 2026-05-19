//! Stage 8.5 — privacy enforcement, per-file span attribution, local-path
//! dependencies.

use std::path::PathBuf;

use axon_project::{LoadedProject, Manifest};

fn make_temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage85-{name}-{pid}-{ts}"));
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
// Per-file spans
// ===========================================================================

#[test]
fn each_module_gets_a_unique_file_id() {
    let dir = make_temp_dir("file_ids");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/a.ax"), "pub fn aa() -> Int { 1 }");
    write(&dir.join("src/b.ax"), "pub fn bb() -> Int { 2 }");
    let p = LoadedProject::load(&dir).unwrap();
    assert_eq!(p.sources.len(), 2);
    let ids: Vec<u16> = p.modules.iter().map(|m| m.source.id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "each module must get a unique id");
}

#[test]
fn spans_from_a_module_carry_its_file_id() {
    let dir = make_temp_dir("span_files");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(
        &dir.join("src/lib.ax"),
        "pub fn id(x: Int) -> Int { x }",
    );
    let p = LoadedProject::load(&dir).unwrap();
    let module = &p.modules[0];
    let expected_id = module.source.id();
    // The first item's span carries the module's file id.
    let item = module.program.items.first().unwrap();
    let span = match item {
        axon_ast::Item::Fn(f) => f.span,
        _ => panic!("expected fn"),
    };
    assert_eq!(span.file, expected_id);
}

// ===========================================================================
// Privacy
// ===========================================================================

#[test]
fn use_of_pub_item_is_allowed() {
    let dir = make_temp_dir("pub_ok");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/lib.ax"), "pub fn ok() -> Int { 1 }");
    write(&dir.join("src/main.ax"), "use lib.{ok}\nfn main() -> Int { ok() }");
    let p = LoadedProject::load(&dir).unwrap();
    let priv_diags: Vec<_> = p
        .diagnostics
        .iter()
        .filter(|d| d.code == Some("P0010"))
        .collect();
    assert!(priv_diags.is_empty(), "got diags: {:#?}", p.diagnostics);
}

#[test]
fn use_of_private_item_emits_p0010() {
    let dir = make_temp_dir("p0010");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(
        &dir.join("src/lib.ax"),
        "fn secret() -> Int { 42 }",
    );
    write(&dir.join("src/main.ax"), "use lib.{secret}\nfn main() -> Int { 0 }");
    let p = LoadedProject::load(&dir).unwrap();
    let priv_diags: Vec<_> = p
        .diagnostics
        .iter()
        .filter(|d| d.code == Some("P0010"))
        .collect();
    assert_eq!(priv_diags.len(), 1, "got diags: {:#?}", p.diagnostics);
    let d = priv_diags[0];
    assert!(d.message.contains("secret"));
    assert!(d.message.contains("not `pub`"));
}

#[test]
fn use_of_unknown_item_emits_p0011() {
    let dir = make_temp_dir("p0011");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    write(&dir.join("src/lib.ax"), "pub fn known() -> Int { 1 }");
    write(
        &dir.join("src/main.ax"),
        "use lib.{not_there}\nfn main() -> Int { 0 }",
    );
    let p = LoadedProject::load(&dir).unwrap();
    let diags: Vec<_> = p
        .diagnostics
        .iter()
        .filter(|d| d.code == Some("P0011"))
        .collect();
    assert_eq!(diags.len(), 1, "got diags: {:#?}", p.diagnostics);
}

#[test]
fn within_same_module_pub_is_irrelevant() {
    let dir = make_temp_dir("intra_mod");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    // No `pub` — but it's called from the same file.
    write(
        &dir.join("src/main.ax"),
        "fn helper() -> Int { 1 }\nfn main() -> Int { helper() }",
    );
    let p = LoadedProject::load(&dir).unwrap();
    let priv_diags: Vec<_> = p
        .diagnostics
        .iter()
        .filter(|d| d.code == Some("P0010"))
        .collect();
    assert!(priv_diags.is_empty());
}

// ===========================================================================
// Local-path dependencies
// ===========================================================================

#[test]
fn local_path_dep_modules_are_loaded_under_the_dep_name() {
    let dep = make_temp_dir("dep_helpers");
    write(
        &dep.join("axon.toml"),
        "[package]\nname = \"helpers\"\nversion = \"0.0.0\"\n",
    );
    write(
        &dep.join("src/util.ax"),
        "pub fn double(n: Int) -> Int { n * 2 }",
    );

    let main = make_temp_dir("dep_main");
    let manifest = format!(
        "[package]\nname = \"main\"\nversion = \"0.0.0\"\n\n[deps.helpers]\npath = \"{}\"\n",
        dep.display()
    );
    write(&main.join("axon.toml"), &manifest);
    write(&main.join("src/main.ax"), "fn main() -> Int { 0 }");

    let p = LoadedProject::load(&main).expect("load");
    let helpers_util = p
        .modules
        .iter()
        .find(|m| m.module_path == "helpers.util")
        .expect("helpers.util module loaded from dep");
    assert!(helpers_util
        .program
        .items
        .iter()
        .any(|i| matches!(i, axon_ast::Item::Fn(f) if f.name.name == "double")));
}

#[test]
fn missing_dep_path_is_a_clear_error() {
    let main = make_temp_dir("dep_missing");
    write(
        &main.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n\n[deps.ghost]\npath = \"./does_not_exist\"\n",
    );
    write(&main.join("src/main.ax"), "fn main() -> Int { 0 }");
    let err = match LoadedProject::load(&main) {
        Ok(_) => panic!("expected error for missing dep"),
        Err(e) => e,
    };
    assert!(err.contains("ghost") || err.contains("does_not_exist"), "msg = {err}");
}

#[test]
fn manifest_parses_deps_section() {
    let toml = r#"
[package]
name = "x"
version = "0.1.0"

[deps.helpers]
path = "../helpers"

[deps.tools]
path = "/abs/path/tools"
"#;
    let m = Manifest::parse(toml).unwrap();
    assert_eq!(m.deps.len(), 2);
    assert_eq!(m.deps["helpers"].path, "../helpers");
    assert_eq!(m.deps["tools"].path, "/abs/path/tools");
}
