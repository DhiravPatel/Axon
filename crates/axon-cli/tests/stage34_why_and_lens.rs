//! Stage 34 — end-to-end coverage for `axon why <EFFECT>` and the
//! effect-row LSP code lens.
//!
//! The why-command tests drive the binary; the effect-lens tests
//! exercise the pure Rust module (the LSP message loop itself doesn't
//! have a JSON-RPC test harness yet).

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
    p.push(format!("axon-stage34why-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p
}

#[test]
fn why_traces_simple_chain_to_a_builtin_leaf() {
    build_axon();
    let dir = temp_dir("simple_chain");
    let path = write(
        &dir,
        "p.ax",
        "fn fetch_html(url: String) -> String uses { Net } { http_fetch(url) }\n\
         fn main() uses { Console, Net } {\n\
         \x20   let html = fetch_html(\"https://example.com\")\n\
         \x20   print(html)\n\
         }\n",
    );
    let out = Command::new(axon_bin())
        .args(["why", "Net", path.to_str().unwrap()])
        .output()
        .expect("axon why");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("why Net in main"));
    assert!(stdout.contains("fetch_html()"), "stdout: {stdout}");
    assert!(stdout.contains("http_fetch()"), "stdout: {stdout}");
    assert!(stdout.contains("built-in requires Net"));
}

#[test]
fn why_prunes_branches_that_dont_carry_the_effect() {
    build_axon();
    let dir = temp_dir("branch_prune");
    let path = write(
        &dir,
        "p.ax",
        "fn pure() -> Int { 1 }\n\
         fn fetch() uses { Net } { http_fetch(\"x\") }\n\
         fn main() uses { Console, Net } { print_int(pure()) fetch() }\n",
    );
    let out = Command::new(axon_bin())
        .args(["why", "Net", path.to_str().unwrap()])
        .output()
        .expect("axon why");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // `pure()` is in main's body but has no Net → must be pruned.
    assert!(
        !stdout.contains("pure()"),
        "pure() should be pruned: {stdout}"
    );
    assert!(stdout.contains("fetch()"), "stdout: {stdout}");
}

#[test]
fn why_reports_cleanly_when_effect_is_not_in_the_graph() {
    build_axon();
    let dir = temp_dir("no_effect");
    let path = write(
        &dir,
        "p.ax",
        "fn main() uses { Console } { print(\"hi\") }\n",
    );
    let out = Command::new(axon_bin())
        .args(["why", "Memory", path.to_str().unwrap()])
        .output()
        .expect("axon why");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("not in main's effect graph"),
        "stdout: {stdout}"
    );
}

#[test]
fn why_accepts_explicit_entry_via_from_flag() {
    build_axon();
    let dir = temp_dir("from_entry");
    let path = write(
        &dir,
        "p.ax",
        "fn worker() uses { Net } { http_fetch(\"x\") }\n\
         fn main() uses { Console } { print(\"hi\") }\n",
    );
    // `axon why Net` from `main` would error (Net not in main's graph),
    // but from `worker` it traces fine.
    let out = Command::new(axon_bin())
        .args([
            "why",
            "Net",
            path.to_str().unwrap(),
            "--from",
            "worker",
        ])
        .output()
        .expect("axon why --from");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("why Net in worker"));
    assert!(stdout.contains("http_fetch()"));
}

#[test]
fn why_help_message_explains_usage() {
    build_axon();
    let out = Command::new(axon_bin())
        .args(["why", "--help"])
        .output()
        .expect("axon why --help");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("usage: axon why"));
    assert!(stdout.contains("EFFECT"));
    assert!(stdout.contains("--from"));
}

// ---------------------------------------------------------------------------
// Effect-row code lens — pure-module checks (the LSP server wiring is
// covered by the cost_lens / code_actions module tests).
// ---------------------------------------------------------------------------

mod effect_lens_smoke {
    use axon_diag::SourceFile;
    use axon_lsp::effect_lens::{lenses_for, EffectLensStatus};

    fn analyze(src: &str) -> (axon_ast::Program, axon_tyck::Ctx) {
        let file = SourceFile::new("t.ax", src.to_string());
        let (p, diags) = axon_parser::parse(&file);
        assert!(diags.is_empty(), "{diags:#?}");
        let (ctx, _td) = axon_tyck::check(&file, &p);
        (p, ctx)
    }

    #[test]
    fn derived_label_includes_inferred_effects() {
        let (p, ctx) = analyze(
            "fn main() { print(\"hi\")\n  print(read_file(\"r\")) }\n",
        );
        let lens = &lenses_for(&p, &ctx)[0];
        assert!(matches!(lens.status, EffectLensStatus::Derived));
        // Inferred row should contain both Console and Fs.Read.
        assert!(lens.label.contains("Console"));
        assert!(lens.label.contains("Fs.Read"));
        assert!(lens.label.contains("derived"));
    }

    #[test]
    fn matches_status_when_declared_row_equals_inferred_set() {
        let (p, ctx) = analyze(
            "fn main() uses { Console, Fs.Read } { print(read_file(\"r\")) }\n",
        );
        let lens = &lenses_for(&p, &ctx)[0];
        assert_eq!(lens.status, EffectLensStatus::Matches);
        assert!(lens.label.contains("matches declaration"));
    }

    #[test]
    fn over_declared_status_lists_unused_atoms() {
        let (p, ctx) = analyze(
            "fn main() uses { Console, Net } { print(\"hi\") }\n",
        );
        let lens = &lenses_for(&p, &ctx)[0];
        match &lens.status {
            EffectLensStatus::OverDeclared { unused } => {
                assert_eq!(unused, &vec!["Net".to_string()]);
            }
            other => panic!("expected OverDeclared, got {other:?}"),
        }
    }
}
