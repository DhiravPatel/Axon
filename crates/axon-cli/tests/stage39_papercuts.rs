//! Stage 39 — acceptance tests for the ease-of-use papercut cleanup.
//!
//! Each test pins one of the papercuts from PAPERCUTS.md that this stage
//! closes: P1 (`let mut` → `var` fix), P3a (trailing-operator line
//! continuation), P4 (`for` over String), P5 (keyed mock), P6 (`replay` on a
//! project dir), P8 (`Dyn`/`Any` casing), P9 (Duration `.as_*` methods),
//! P10 (`String.replace`/`.repeat`), P13 (single E0203 for one typo).

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
    p.push(format!("axon-stage39-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(dir: &std::path::Path, src: &str) -> PathBuf {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    path
}

fn run_src(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = write(dir, src);
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

fn check_src(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = write(dir, src);
    Command::new(axon_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("axon check")
}

fn stdout_lines(out: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.to_string())
        .collect()
}

// =========================================================================
// P1 — `let mut x` reports one fixable error and `axon fix` rewrites to `var`
// =========================================================================

#[test]
fn p1_let_mut_reports_single_fixable_error() {
    build_axon();
    let dir = temp_dir("p1_err");
    let out = check_src(
        &dir,
        "fn main() uses { Console } {\n    let mut start: Int = 0\n    start = start + 5\n}\n",
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Exactly one P0001 — the old `let mut` cascade ("expected a pattern" +
    // "expected an expression, got Colon") is gone.
    assert_eq!(
        combined.matches("P0001").count(),
        1,
        "expected exactly one P0001 diagnostic, got:\n{combined}"
    );
    assert!(
        combined.contains("var"),
        "diagnostic should steer the user to `var`:\n{combined}"
    );
    assert!(
        !combined.contains("expected a pattern"),
        "old cascade leaked through:\n{combined}"
    );
}

#[test]
fn p1_axon_fix_rewrites_let_mut_to_var_and_runs() {
    build_axon();
    let dir = temp_dir("p1_fix");
    let path = write(
        &dir,
        "fn main() uses { Console } {\n    let mut start: Int = 0\n    start = start + 5\n    print(\"start = {start}\")\n}\n",
    );
    let out = Command::new(axon_bin())
        .args(["fix", "--apply", path.to_str().unwrap()])
        .output()
        .expect("axon fix");
    assert!(out.status.success(), "fix failed: {:?}", out);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("var start: Int = 0"),
        "fix should produce `var start: Int = 0`, got:\n{after}"
    );
    assert!(!after.contains("let mut"), "`let mut` should be gone:\n{after}");
    // The rewritten file runs and behaves as a mutable binding.
    let run = Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(run.status.success(), "run after fix failed: {:?}", run);
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("start = 5"),
        "expected mutable behavior, got: {}",
        String::from_utf8_lossy(&run.stdout)
    );
}

// =========================================================================
// P3a — a trailing binary operator continues onto the next line
// =========================================================================

#[test]
fn p3a_trailing_operator_continues_line() {
    build_axon();
    let dir = temp_dir("p3a");
    let out = run_src(
        &dir,
        "fn main() uses { Console } {\n\
         \x20   let s = \"a \" +\n            \"b \" +\n            \"c\"\n\
         \x20   print(s)\n\
         \x20   let n = 1 +\n            2 +\n            3\n\
         \x20   print(\"n = {n}\")\n\
         }\n",
    );
    assert!(out.status.success(), "{:?}", out);
    let lines = stdout_lines(&out);
    assert_eq!(lines, ["a b c", "n = 6"], "got: {lines:?}");
}

#[test]
fn p3a_separate_statements_are_not_merged() {
    build_axon();
    let dir = temp_dir("p3a_reg");
    // No trailing operator: the two assignments stay independent statements.
    let out = run_src(
        &dir,
        "fn main() uses { Console } {\n    var a = 10\n    a = a + 1\n    print(\"a = {a}\")\n}\n",
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(stdout_lines(&out), ["a = 11"]);
}

// =========================================================================
// P4 — `for x in` works over String / List / Set / Range
// =========================================================================

#[test]
fn p4_for_over_string_and_collections() {
    build_axon();
    let dir = temp_dir("p4");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    var chars = 0
    for ch in "abc" { chars = chars + 1 }
    print("chars = {chars}")

    var lsum = 0
    for x in [1, 2, 3] { lsum = lsum + x }
    print("lsum = {lsum}")

    var rsum = 0
    for i in 0..5 { rsum = rsum + i }
    print("rsum = {rsum}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        stdout_lines(&out),
        ["chars = 3", "lsum = 6", "rsum = 10"],
        "got: {:?}",
        stdout_lines(&out)
    );
}

// =========================================================================
// P5 — keyed mock returns the right response regardless of call order
// =========================================================================

#[test]
fn p5_keyed_mock_is_order_independent() {
    build_axon();
    let dir = temp_dir("p5");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console, LLM, Net } {
    let m = mock_model("keyed", [
        ["Alice", "reply-for-alice"],
        ["Bob", "reply-for-bob"],
    ])
    // Ask out of declaration order — Bob first.
    let rb = ask m { user: "ticket from Bob" }
    let ra = ask m { user: "ticket from Alice" }
    print(rb)
    print(ra)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        stdout_lines(&out),
        ["reply-for-bob", "reply-for-alice"],
        "keyed mock should match by content, got: {:?}",
        stdout_lines(&out)
    );
}

// =========================================================================
// P6 — `axon replay` accepts a project directory (parity with `axon run`)
// =========================================================================

#[test]
fn p6_replay_accepts_a_project_directory() {
    build_axon();
    let dir = temp_dir("p6");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("axon.toml"),
        "[package]\nname = \"p6\"\nversion = \"0.1.0\"\n\n[run]\nentry = \"main\"\nsrc = \"src\"\n\n[caps]\ndefault = [\"Console\", \"LLM\", \"Net\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.ax"),
        "fn main() uses { Console, LLM, Net } {\n    let m = mock_model(\"fixed\", \"hello\")\n    print(ask m { user: \"hi\" })\n}\n",
    )
    .unwrap();
    let rec = dir.join("rec.json");

    let record = Command::new(axon_bin())
        .args(["run", "--record", rec.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .expect("axon run --record");
    assert!(record.status.success(), "record on dir failed: {:?}", record);
    assert!(rec.exists(), "recording not produced");
    let recorded = String::from_utf8_lossy(&record.stdout).to_string();

    // The key assertion: `axon replay <rec> <dir>` no longer errors with
    // "Is a directory" and reproduces the recorded stdout byte-for-byte.
    let replay = Command::new(axon_bin())
        .args(["replay", rec.to_str().unwrap(), dir.to_str().unwrap()])
        .output()
        .expect("axon replay dir");
    assert!(
        replay.status.success(),
        "replay on dir failed: {:?}",
        replay
    );
    assert_eq!(
        String::from_utf8_lossy(&replay.stdout),
        recorded,
        "replay stdout diverged from recording"
    );
}

// =========================================================================
// P8 — `Dyn` / `dyn` / `Any` all resolve as the gradual type
// =========================================================================

#[test]
fn p8_dyn_any_casing_all_resolve() {
    build_axon();
    let dir = temp_dir("p8");
    let out = check_src(
        &dir,
        "fn main() uses { Console } {\n    let a: List<dyn> = []\n    let b: List<Dyn> = []\n    let c: List<Any> = []\n}\n",
    );
    assert!(
        out.status.success(),
        "Dyn/dyn/Any should all type-check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// =========================================================================
// P9 — Duration `.as_ms()` / `.as_ns()` / `.as_secs()` read it out
// =========================================================================

#[test]
fn p9_duration_unit_methods() {
    build_axon();
    let dir = temp_dir("p9");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let d = dur_from_millis(1500)
    print("ms = {d.as_ms()}")
    print("secs = {d.as_secs()}")
    print("ns = {d.as_ns()}")
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        stdout_lines(&out),
        ["ms = 1500", "secs = 1", "ns = 1500000000"],
        "got: {:?}",
        stdout_lines(&out)
    );
}

// =========================================================================
// P10 — `String.replace` and `.repeat` exist as methods
// =========================================================================

#[test]
fn p10_string_replace_and_repeat_methods() {
    build_axon();
    let dir = temp_dir("p10");
    let out = run_src(
        &dir,
        r#"fn main() uses { Console } {
    let r = "a-b-c".replace("-", "_")
    let x = "ab".repeat(3)
    print(r)
    print(x)
}
"#,
    );
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(stdout_lines(&out), ["a_b_c", "ababab"], "got: {:?}", stdout_lines(&out));
}

// =========================================================================
// P13 — one unknown-type typo reports E0203 exactly once
// =========================================================================

#[test]
fn p13_unknown_type_reported_exactly_once() {
    build_axon();
    let dir = temp_dir("p13_one");
    let out = check_src(&dir, "fn f(x: Bogus) -> Int { 0 }\nfn main() {}\n");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        combined.matches("error[E0203]").count(),
        1,
        "one typo must report E0203 once, got:\n{combined}"
    );
}

#[test]
fn p13_two_distinct_typos_still_report_twice() {
    build_axon();
    let dir = temp_dir("p13_two");
    // Guard against over-dedup: two different bad types at two spans = two E0203.
    let out = check_src(&dir, "fn f(x: Aaa, y: Bbb) -> Int { 0 }\nfn main() {}\n");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        combined.matches("error[E0203]").count(),
        2,
        "two distinct typos must report E0203 twice, got:\n{combined}"
    );
}

// =========================================================================
// P7 — inline record types parse inside generics and as bare annotations
// =========================================================================

#[test]
fn p7_inline_record_type_in_generics() {
    build_axon();
    let dir = temp_dir("p7");
    let out = check_src(
        &dir,
        "fn build(target: Model) -> List<{ target: Model, user: String }> uses { LLM } { [] }\nfn main() {}\n",
    );
    assert!(
        out.status.success(),
        "inline record type in a generic should type-check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
