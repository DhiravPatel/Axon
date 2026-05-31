//! Stage 35 verification-fixes regression tests.
//!
//! Each test pins one finding from the §35.6 adversarial pass. If any
//! of these regresses we want the suite to go red loudly.

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
    p.push(format!("axon-stage35vf-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn project(dir: &std::path::Path, main_src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("axon.toml"),
        "[package]\nname = \"v\"\nversion = \"0.1.0\"\n[run]\nentry = \"main\"\nsrc = \"src\"\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\", \"Memory\", \"Tool\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.ax"), main_src).unwrap();
}

fn write(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p
}

// =========================================================================
// C2 — --match-trajectory must flag a regression that removes model calls.
// =========================================================================

#[test]
fn match_trajectory_flags_drift_to_zero_model_calls() {
    build_axon();
    let dir = temp_dir("c2_zero_drift");
    project(
        &dir,
        "fn ask_it() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"hi\")\n\
         \x20   ask m { user: \"q\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"t\" { let _ = ask_it() }\n",
    );
    // Record the baseline with a model call.
    let record = Command::new(axon_bin())
        .args(["test", "--record-trajectory", "base", dir.to_str().unwrap()])
        .output()
        .expect("record");
    assert!(record.status.success(), "{:?}", record);

    // Mutate the test to no longer make a model call.
    std::fs::write(
        dir.join("src/main.ax"),
        "fn ask_it() -> String { \"no model now\" }\n\
         fn main() uses { Console } {}\n\
         test \"t\" { let _ = ask_it() }\n",
    )
    .unwrap();

    // Match should DRIFT, not silently pass.
    let matched = Command::new(axon_bin())
        .args(["test", "--match-trajectory", "base", dir.to_str().unwrap()])
        .output()
        .expect("match");
    let stdout = String::from_utf8_lossy(&matched.stdout);
    assert!(
        stdout.contains("DRIFT") || stdout.contains("drifted"),
        "expected drift report when model call disappears: {stdout}"
    );
    // Exit non-zero so CI catches the regression.
    assert!(
        !matched.status.success(),
        "match must exit non-zero on drift"
    );
}

#[test]
fn match_trajectory_treats_missing_baseline_as_drift() {
    build_axon();
    let dir = temp_dir("c2_missing");
    project(
        &dir,
        "fn ask_it() -> String uses { LLM, Net } {\n\
         \x20   let m = mock_model(\"fixed\", \"x\")\n\
         \x20   ask m { user: \"q\" }\n\
         }\n\
         fn main() uses { Console } {}\n\
         test \"t\" { let _ = ask_it() }\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--match-trajectory", "missing", dir.to_str().unwrap()])
        .output()
        .expect("axon test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no saved trajectory"),
        "expected missing-baseline note: {stdout}"
    );
    assert!(
        !out.status.success(),
        "missing baseline in match mode must exit non-zero"
    );
}

// =========================================================================
// M2 — snapshot-name path traversal refused.
// =========================================================================

#[test]
fn record_trajectory_rejects_path_traversal_in_set_name() {
    build_axon();
    let dir = temp_dir("m2_traversal");
    project(
        &dir,
        "fn main() uses { Console } {}\n\
         test \"t\" { let x = 1 }\n",
    );
    for name in ["../../etc", "/tmp/escape", "../poisoned", ".."] {
        let out = Command::new(axon_bin())
            .args(["test", "--record-trajectory", name, dir.to_str().unwrap()])
            .output()
            .expect("axon test");
        assert!(
            !out.status.success(),
            "snapshot-set name `{name}` should be rejected"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("rejected"),
            "expected refusal message for `{name}`: {stderr}"
        );
    }
}

// =========================================================================
// C1 — slot vs state-field precedence: user state wins.
// =========================================================================

#[test]
fn c1_slot_does_not_shadow_user_state_field_of_same_name() {
    build_axon();
    let dir = temp_dir("c1");
    let path = write(
        &dir,
        "p.ax",
        "tool a(s: String) -> String uses { Net } { s }\n\
         tool b(s: String) -> String uses { Net } { s }\n\
         agent T() {\n\
         \x20   uses_tools: [a, b],\n\
         \x20   state tools: List<dyn> = []\n\
         \x20   on count() -> Int { list_len(self.tools) }\n\
         }\n\
         fn main() uses { Spawn, Console } { let t = spawn T(); print_int(t.count()) }\n",
    );
    let out = Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim_end().ends_with("0"),
        "user state field must win (expected 0): {stdout}"
    );
}

// =========================================================================
// C3 — doc-test brace injection refused.
// =========================================================================

#[test]
fn c3_doc_snippet_with_unbalanced_braces_is_refused() {
    build_axon();
    let dir = temp_dir("c3");
    project(
        &dir,
        "/// Try to break out:\n\
         /// ```axon\n\
         /// let x = 1\n\
         /// } tool pwn(s: String) -> String uses { Net } { s }\n\
         /// test \"injected\" { assert(false)\n\
         /// ```\n\
         pub fn safe() -> Int { 0 }\n\
         fn main() uses { Console } {}\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The synthesizer emits a failing test stub whose name signals the
    // refusal — neither the injected `pwn` tool nor the injected
    // `"injected"` test must appear in the run.
    assert!(
        stdout.contains("brace_imbalance") || stdout.contains("FAIL"),
        "expected refusal stub: {stdout}"
    );
    assert!(
        !stdout.contains("\"injected\""),
        "injected test must not appear in the run: {stdout}"
    );
}

// =========================================================================
// M1 — slot expressions are typechecked.
// =========================================================================

#[test]
fn m1_slot_expression_with_wrong_type_is_a_typeck_error() {
    build_axon();
    let dir = temp_dir("m1");
    // `memory: 42` — Int where Memory is expected. Without M1 this
    // silently typechecked and crashed at runtime; with M1 tyck flags it.
    let path = write(
        &dir,
        "p.ax",
        "agent T() {\n\
         \x20   memory: 42\n\
         \x20   on noop() {}\n\
         }\n\
         fn main() uses { Spawn, Console } { let _ = spawn T() }\n",
    );
    let out = Command::new(axon_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("axon check");
    assert!(
        !out.status.success(),
        "memory: 42 should typecheck-error: {:?}",
        out
    );
}

// =========================================================================
// M3 — doc-test collision with user test is suffixed, not E0204.
// =========================================================================

#[test]
fn m3_doc_test_name_collision_is_suffixed_not_e0204() {
    build_axon();
    let dir = temp_dir("m3");
    project(
        &dir,
        "/// ```axon\n\
         /// let r = add(1, 2)\n\
         /// ```\n\
         fn add(a: Int, b: Int) -> Int { a + b }\n\
         fn main() uses { Console } {}\n\
         test \"doc(main.add)\" { let _ = add(2, 3) }\n",
    );
    let out = Command::new(axon_bin())
        .args(["test", "--doc", dir.to_str().unwrap()])
        .output()
        .expect("axon test --doc");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Should NOT produce an E0204 pointing at <doc-tests>.
    assert!(
        !combined.contains("E0204"),
        "synthesizer must not produce an E0204 on user-test name collision: {combined}"
    );
    assert!(
        out.status.success(),
        "both tests should run: {:?}",
        out
    );
}

// =========================================================================
// M5 — axon test --help exists.
// =========================================================================

#[test]
fn m5_axon_test_help_lists_stage35_flags() {
    build_axon();
    let out = Command::new(axon_bin())
        .args(["test", "--help"])
        .output()
        .expect("axon test --help");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("usage: axon test"), "{stdout}");
    assert!(stdout.contains("--doc"), "{stdout}");
    assert!(stdout.contains("--record-trajectory"), "{stdout}");
    assert!(stdout.contains("--match-trajectory"), "{stdout}");
}
