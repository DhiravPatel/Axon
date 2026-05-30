//! Stage 32 — `axon fix` end-to-end.
//!
//! Each test runs the actual `axon` binary against a temp file, parses
//! stdout / inspects the rewritten file, and asserts on what changed.
//! No mocking — this is the same path a real user hits at the shell.

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
    p.push(format!("axon-stage32fix-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write_src(dir: &std::path::Path, src: &str) -> PathBuf {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    path
}

fn run_fix(path: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["fix"];
    args.extend_from_slice(extra);
    args.push(path.to_str().unwrap());
    Command::new(axon_bin())
        .args(&args)
        .output()
        .expect("axon fix")
}

// ===========================================================================
// Did-you-mean (E0202) — local typo.
// ===========================================================================

#[test]
fn dry_run_proposes_replacement_for_local_typo() {
    build_axon();
    let dir = temp_dir("e0202_dry");
    let path = write_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let greeting = \"hi\"\n\
         \x20   print(greetng)\n\
         }\n",
    );
    let out = run_fix(&path, &[]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fix [E0202]"),
        "stdout missing E0202 label: {stdout}"
    );
    assert!(
        stdout.contains("replace `greetng` with `greeting`"),
        "stdout missing replacement description: {stdout}"
    );
    // Dry run must not touch the file.
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("greetng"), "file was modified during dry run");
}

#[test]
fn apply_rewrites_local_typo_in_place_and_check_then_passes() {
    build_axon();
    let dir = temp_dir("e0202_apply");
    let path = write_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let greeting = \"hi\"\n\
         \x20   print(greetng)\n\
         }\n",
    );
    let out = run_fix(&path, &["--apply"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("print(greeting)"),
        "expected typo to be fixed: {after}"
    );
    // The file should now type-check.
    let check = Command::new(axon_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(check.status.success(), "{:?}", check);
}

// ===========================================================================
// Missing effect (E0210) — insert into `uses { ... }`.
// ===========================================================================

#[test]
fn apply_inserts_missing_effect_into_existing_uses_row() {
    build_axon();
    let dir = temp_dir("e0210_existing");
    let path = write_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   print(read_file(\"/etc/hosts\"))\n\
         }\n",
    );
    let out = run_fix(&path, &["--apply"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    // Clean spacing: `Console, Fs.Read`, not `Console , Fs.Read` or similar.
    assert!(
        after.contains("uses { Console, Fs.Read }"),
        "expected effect inserted cleanly: {after}"
    );
}

#[test]
fn apply_synthesizes_uses_row_when_function_has_none() {
    build_axon();
    let dir = temp_dir("e0210_synth");
    // No `uses` clause at all — the fix synthesizes one before `{`.
    let path = write_src(
        &dir,
        "fn main() {\n\
         \x20   print(\"hi\")\n\
         }\n",
    );
    let out = run_fix(&path, &["--apply"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("uses { Console }"),
        "expected synthesized uses row: {after}"
    );
    // And it must still type-check.
    let check = Command::new(axon_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(check.status.success(), "{:?}", check);
}

// ===========================================================================
// Did-you-mean (E0203) — unknown type.
// ===========================================================================

#[test]
fn apply_fixes_type_typo_against_a_user_defined_schema() {
    build_axon();
    let dir = temp_dir("e0203");
    let path = write_src(
        &dir,
        "schema Profile { name: String, age: Int }\n\
         fn lookup(p: Profle) -> String { p.name }\n\
         fn main() uses { Console } { print(\"ok\") }\n",
    );
    let out = run_fix(&path, &["--apply"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("p: Profile"),
        "expected type typo to be fixed: {after}"
    );
}

// ===========================================================================
// --only restricts to one code.
// ===========================================================================

#[test]
fn only_flag_restricts_to_one_diagnostic_code() {
    build_axon();
    let dir = temp_dir("only");
    // Has both an E0202 typo and an E0210 missing effect.
    let path = write_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let greeting = \"hi\"\n\
         \x20   print(greetng)\n\
         \x20   print(read_file(\"/etc/hosts\"))\n\
         }\n",
    );
    let out = run_fix(&path, &["--apply", "--only", "E0210"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    // Only the effect row should have changed.
    assert!(
        after.contains("uses { Console, Fs.Read }"),
        "E0210 fix should apply: {after}"
    );
    // The typo must NOT have been touched.
    assert!(
        after.contains("print(greetng)"),
        "E0202 fix must be filtered out by --only E0210: {after}"
    );
}

// ===========================================================================
// Idempotence: rerunning `axon fix --apply` on a clean file is a no-op.
// ===========================================================================

#[test]
fn rerunning_on_clean_file_is_a_noop() {
    build_axon();
    let dir = temp_dir("idem");
    let path = write_src(
        &dir,
        "fn main() uses { Console } { print(\"hi\") }\n",
    );
    let before = std::fs::read_to_string(&path).unwrap();
    let out = run_fix(&path, &["--apply"]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        before, after,
        "fix --apply on a clean file must not alter it"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("nothing to fix") || stdout.contains("none carry"),
        "expected a 'nothing to fix' message: {stdout}"
    );
}

// ===========================================================================
// Unknown name with no close candidate → no fix attached.
// ===========================================================================

#[test]
fn typo_with_no_close_candidate_does_not_produce_a_fix() {
    build_axon();
    let dir = temp_dir("no_candidate");
    let path = write_src(
        &dir,
        // `xyzqq` has nothing within edit distance 2 → no fix should be
        // attached, so axon fix produces no edits.
        "fn main() uses { Console } { print(xyzqq) }\n",
    );
    let out = run_fix(&path, &[]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("none carry"),
        "expected 'none carry a fix' since no close candidate exists: {stdout}"
    );
}
