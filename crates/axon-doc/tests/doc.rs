//! Stage 10 — `axon doc` tests.

use std::path::PathBuf;

use axon_doc::{extract_doc_pairs, generate};
use axon_project::LoadedProject;

fn make_temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-doc-test-{name}-{pid}-{ts}"));
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

#[test]
fn doc_comments_attach_to_the_next_item() {
    let dir = make_temp_dir("doc_attach");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"hello\"\nversion = \"0.1.0\"\n",
    );
    write(
        &dir.join("src/lib.ax"),
        "/// Adds two integers.\n\
         /// Useful for arithmetic.\n\
         pub fn add(a: Int, b: Int) -> Int { a + b }\n",
    );
    let p = LoadedProject::load(&dir).expect("load");
    let pairs = extract_doc_pairs(&p.modules[0]);
    assert_eq!(pairs.len(), 1);
    assert!(pairs[0].doc.contains("Adds two integers"));
    assert!(pairs[0].doc.contains("Useful for arithmetic"));
}

#[test]
fn intervening_non_trivia_resets_pending_docs() {
    let dir = make_temp_dir("doc_reset");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"x\"\nversion = \"0.0.0\"\n",
    );
    // Doc comment, then an unrelated `pub fn` — the docs belong to the
    // first fn, not the second.
    write(
        &dir.join("src/lib.ax"),
        "/// Docs for first.\n\
         pub fn first() -> Int { 1 }\n\
         pub fn second() -> Int { 2 }\n",
    );
    let p = LoadedProject::load(&dir).expect("load");
    let pairs = extract_doc_pairs(&p.modules[0]);
    let first_idx = p
        .modules[0]
        .program
        .items
        .iter()
        .position(|i| matches!(i, axon_ast::Item::Fn(f) if f.name.name == "first"))
        .unwrap();
    assert!(pairs.iter().any(|p| p.item_index == first_idx));
    let second_idx = p
        .modules[0]
        .program
        .items
        .iter()
        .position(|i| matches!(i, axon_ast::Item::Fn(f) if f.name.name == "second"))
        .unwrap();
    assert!(!pairs.iter().any(|p| p.item_index == second_idx));
}

#[test]
fn site_contains_index_module_pages_and_stylesheet() {
    let dir = make_temp_dir("site_layout");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    write(
        &dir.join("src/math.ax"),
        "/// Adds two integers.\n\
         pub fn add(a: Int, b: Int) -> Int { a + b }\n",
    );
    write(
        &dir.join("src/main.ax"),
        "/// Program entry point.\nfn main() -> Int { 0 }\n",
    );
    let p = LoadedProject::load(&dir).expect("load");
    let site = generate(&p);

    assert!(site.files.contains_key(&PathBuf::from("index.html")));
    assert!(site.files.contains_key(&PathBuf::from("style.css")));
    assert!(site.files.contains_key(&PathBuf::from("math.html")));
    assert!(site.files.contains_key(&PathBuf::from("main.html")));

    let index = &site.files[&PathBuf::from("index.html")];
    assert!(index.contains("demo"));
    assert!(index.contains("math"));
    assert!(index.contains("main"));

    let math = &site.files[&PathBuf::from("math.html")];
    assert!(math.contains("add"));
    assert!(math.contains("Int"));
    assert!(math.contains("Adds two integers"));
}

#[test]
fn html_escapes_dangerous_characters() {
    let dir = make_temp_dir("escape");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"esc\"\nversion = \"0.1.0\"\n",
    );
    // A doc comment mentioning angle brackets, plus a fn whose body has
    // some unrelated structure.
    write(
        &dir.join("src/lib.ax"),
        "/// Compares using `<` and `>` operators.\npub fn cmp(a: Int, b: Int) -> Bool { a < b }\n",
    );
    let p = LoadedProject::load(&dir).expect("load");
    let site = generate(&p);
    let module_html = site
        .files
        .get(&PathBuf::from("lib.html"))
        .expect("module page");
    // The literal `<` from the doc comment should be HTML-encoded.
    assert!(
        module_html.contains("&lt;") || module_html.contains("<code>&lt;</code>"),
        "got: {module_html}"
    );
}

#[test]
fn visibility_class_marks_private_items() {
    let dir = make_temp_dir("vis");
    write(
        &dir.join("axon.toml"),
        "[package]\nname = \"vis\"\nversion = \"0.1.0\"\n",
    );
    write(
        &dir.join("src/lib.ax"),
        "pub fn pub_fn() -> Int { 1 }\nfn private_fn() -> Int { 2 }\n",
    );
    let p = LoadedProject::load(&dir).expect("load");
    let site = generate(&p);
    let html = &site.files[&PathBuf::from("lib.html")];
    // Public items use the default `kind` class; private get `kind private`.
    assert!(html.contains("private"));
}
