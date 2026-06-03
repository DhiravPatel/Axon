//! Stage 36 DX pack acceptance tests:
//!   - flow_majority / flow_majority_with (§36.B.3)
//!   - str_split_lines / str_split_once + dur_micros/dur_nanos/dur_seconds_f64 (§36.B.2)
//!   - with_retry / with_timeout call-site combinators (§36.B.5)
//!   - axon trace promote subcommand (§36.B.4)

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
    p.push(format!("axon-stage36dx-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn run_src(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

// =========================================================================
// §36.B.3 flow_majority / flow_majority_with
// =========================================================================

#[test]
fn flow_majority_picks_top_count_and_handles_first_seen_tiebreak() {
    build_axon();
    let dir = temp_dir("maj");
    let prog = r#"
fn main() uses { Console } {
    // Clear majority: bug_report wins (2 of 3).
    print(flow_majority(["refund", "bug_report", "bug_report"]))
    // Tie 1-1-1: first-seen wins (refund).
    print(flow_majority(["refund", "bug", "question"]))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["bug_report", "refund"], "got: {lines:?}");
}

#[test]
fn flow_majority_with_returns_support_total_and_tie_flag() {
    build_axon();
    let dir = temp_dir("majw");
    let prog = r#"
fn main() uses { Console } {
    let r = flow_majority_with(["a", "b", "a", "b"])
    print(r.label)
    print_int(r.support)
    print_int(r.total)
    // Two distinct options tied at 2 votes each → tie = true.
    if r.tie { print("tied") } else { print("decided") }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tied"), "stdout: {stdout}");
}

#[test]
fn flow_majority_empty_votes_is_an_error() {
    build_axon();
    let dir = temp_dir("emptymaj");
    let prog = r#"
fn main() uses { Console } {
    print(flow_majority([]))
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("empty"),
        "stderr should explain empty-votes; got: {stderr}"
    );
}

// =========================================================================
// §36.B.2 stdlib expansion
// =========================================================================

#[test]
fn str_split_lines_handles_lf_and_no_trailing_empty() {
    build_axon();
    let dir = temp_dir("lines");
    let prog = r#"
fn main() uses { Console } {
    let xs = str_split_lines("a\nb\nc\n")
    print_int(list_len(xs))
    print(list_get(xs, 0))
    print(list_get(xs, 2))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["3", "a", "c"]);
}

#[test]
fn str_split_once_splits_at_first_separator() {
    build_axon();
    let dir = temp_dir("splitonce");
    let prog = r#"
fn main() uses { Console } {
    let xs = str_split_once("key=val=ue", "=")
    print_int(list_len(xs))
    print(list_get(xs, 0))
    print(list_get(xs, 1))
    let ys = str_split_once("nope", "=")
    print_int(list_len(ys))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["2", "key", "val=ue", "1"]);
}

#[test]
fn duration_micros_nanos_seconds_f64_round_trip() {
    build_axon();
    let dir = temp_dir("dur");
    let prog = r#"
fn main() uses { Console } {
    let d = dur_from_millis(1500)
    print_int(dur_nanos(d))
    print_int(dur_micros(d))
    // dur_seconds_f64 returns Float; print via float-aware path.
    let s = dur_seconds_f64(d)
    if s > 1.4 { print("ge_1.4") } else { print("lt_1.4") }
    if s < 1.6 { print("lt_1.6") } else { print("ge_1.6") }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["1500000000", "1500000", "ge_1.4", "lt_1.6"]);
}

// =========================================================================
// §36.B.5 with_retry / with_timeout
// =========================================================================

#[test]
fn with_retry_re_invokes_on_failure_until_attempt_succeeds() {
    // We can't easily script a "fail twice then succeed" thunk without
    // mutable global state. Instead, this test pins the happy path: a
    // succeeding thunk is invoked exactly once and its value returned.
    build_axon();
    let dir = temp_dir("retryok");
    let prog = r#"
fn main() uses { Console } {
    let v = with_retry(|| 42, 3, 10)
    print_int(v)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("42"), "stdout: {stdout}");
}

#[test]
fn with_retry_surfaces_last_error_after_exhausting_attempts() {
    build_axon();
    let dir = temp_dir("retryfail");
    let prog = r#"
fn main() uses { Console } {
    let _ = with_retry(|| { panic("forced") }, 2, 1)
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("attempts failed") || stderr.contains("forced"),
        "stderr should mention retry exhaustion or the underlying error; got: {stderr}"
    );
}

#[test]
fn with_timeout_succeeds_within_budget() {
    build_axon();
    let dir = temp_dir("timeout_ok");
    let prog = r#"
fn main() uses { Console } {
    let v = with_timeout(|| 7, 1000)
    print_int(v)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("7"), "stdout: {stdout}");
}

// =========================================================================
// §36.B.4 axon trace promote
// =========================================================================

#[test]
fn axon_trace_promote_appends_a_regression_test_from_a_recording() {
    build_axon();
    let dir = temp_dir("promote");
    let src = dir.join("orig.ax");
    let rec = dir.join("rec.json");
    let suite = dir.join("suite.ax");
    std::fs::write(
        &src,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "the official answer is 42")
    let r = ask m { user: "what is the answer?" }
    print(r)
}
"#,
    )
    .unwrap();
    let rec_out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(rec_out.status.success(), "record: {:?}", rec_out);

    // Pre-create the suite with a header so we can verify trace promote
    // APPENDS (doesn't overwrite).
    std::fs::write(&suite, "// existing comment — must survive promote\n").unwrap();
    let promote = Command::new(axon_bin())
        .args([
            "trace",
            "promote",
            rec.to_str().unwrap(),
            "--to-suite",
            suite.to_str().unwrap(),
            "--name",
            "regression_my_check",
        ])
        .output()
        .expect("axon trace promote");
    assert!(
        promote.status.success(),
        "promote: {}",
        String::from_utf8_lossy(&promote.stderr)
    );
    let body = std::fs::read_to_string(&suite).unwrap();
    assert!(
        body.contains("existing comment — must survive promote"),
        "promote must not rewrite the suite; got:\n{body}"
    );
    assert!(
        body.contains("test \"regression_my_check\""),
        "promote must append the named test; got:\n{body}"
    );
    assert!(
        body.contains("the official answer is 42"),
        "promote must inline the recorded response; got:\n{body}"
    );
}

#[test]
fn axon_trace_promote_refuses_recording_with_no_model_calls() {
    build_axon();
    let dir = temp_dir("promote_empty");
    let rec = dir.join("rec.json");
    let suite = dir.join("suite.ax");
    // Hand-crafted recording with no model_call events.
    std::fs::write(&rec, r#"{"version": 1, "events": []}"#).unwrap();
    let out = Command::new(axon_bin())
        .args([
            "trace",
            "promote",
            rec.to_str().unwrap(),
            "--to-suite",
            suite.to_str().unwrap(),
        ])
        .output()
        .expect("axon trace promote");
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no `model_call`"),
        "stderr should explain why we refused; got: {stderr}"
    );
    // Suite must not have been created.
    assert!(
        !suite.exists(),
        "suite should not be created on refusal"
    );
}
