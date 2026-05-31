//! Stage 36 §36.6 verification-fix regression tests.
//!
//! Each test pins one finding from the §36.6 adversarial pass. If any
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
    p.push(format!("axon-stage36vf-{name}-{}-{ts}", std::process::id()));
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
// C1 — parallel{} record/replay byte identity holds when an arm errors.
// (Reviewer found: parallel returned early on first failed arm without
// recording subsequent successes, so replay's pop attributed responses
// to the wrong arm.)
// =========================================================================

#[test]
fn c1_failed_parallel_arm_does_not_corrupt_recording() {
    build_axon();
    let dir = temp_dir("c1");
    let rec = dir.join("rec.json");
    // Three arms: two succeed (mock_model_slow), one fails (mock_model
    // with empty content list — provider returns ProviderError). Since
    // mock_model_slow returns the same text each call, we use a script
    // model that errors after one call. Simpler: rely on the recording
    // being EMPTY when ANY arm fails, by construction. We verify by
    // counting events in the recording file.
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    // Trigger an arm failure by exhausting the script provider.
    let m_ok = mock_model_slow("ok", 10)
    let m_bad = mock_model("script", [])  // empty script -> immediate error on call
    let _xs = parallel {
        ask m_ok { user: "q1" },
        ask m_bad { user: "q2" },
    }
}
"#;
    let path = dir.join("p.ax");
    std::fs::write(&path, prog).unwrap();
    let out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    // The run errors (one arm fails). What matters for C1: the recording
    // must NOT have a partial set of events that would desync replay.
    let _ = out;
    if rec.exists() {
        let raw = std::fs::read_to_string(&rec).unwrap();
        // Either no recording file at all, or events array is empty.
        // ANY non-zero event count here would mean we recorded the
        // success without the failure, breaking replay.
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let n_events = v
            .get("events")
            .and_then(|e| e.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(
            n_events, 0,
            "C1 regressed: failed parallel batch left {n_events} \
             events in recording; replay would desync. Recording: {raw}"
        );
    }
}

// =========================================================================
// S4 — with_retry `times` is bounded.
// =========================================================================

#[test]
fn s4_with_retry_times_above_1000_is_rejected() {
    build_axon();
    let dir = temp_dir("s4");
    let prog = r#"
fn main() uses { Console } {
    let _ = with_retry(|| 1, 1001, 0)
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("1..=1000") || stderr.contains("must be in"),
        "expected bound error message; got: {stderr}"
    );
}

#[test]
fn s4_with_retry_times_at_boundary_1000_is_accepted() {
    build_axon();
    let dir = temp_dir("s4ok");
    // Run with `times = 1000` and a thunk that succeeds first-try (so
    // the loop returns immediately, the bound is just structural).
    let prog = r#"
fn main() uses { Console } {
    let v = with_retry(|| 7, 1000, 0)
    print_int(v)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains('7'),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// =========================================================================
// S4b — with_retry backoff_ms and with_timeout ms bounded.
// =========================================================================

#[test]
fn s4b_with_retry_backoff_above_1h_is_rejected() {
    build_axon();
    let dir = temp_dir("s4b1");
    let prog = r#"
fn main() uses { Console } {
    let _ = with_retry(|| 1, 1, 3600001)
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("3600000"),
        "expected 1h bound in error message; got: {stderr}"
    );
}

#[test]
fn s4b_with_timeout_above_1h_is_rejected() {
    build_axon();
    let dir = temp_dir("s4b2");
    let prog = r#"
fn main() uses { Console } {
    let _ = with_timeout(|| 1, 3600001)
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("3600000"),
        "expected 1h bound in error message; got: {stderr}"
    );
}

// =========================================================================
// L1 — trace promote auto-name includes filename so identical responses
// from different recordings don't collide.
// =========================================================================

#[test]
fn l1_trace_promote_auto_name_distinguishes_recordings_with_identical_content() {
    build_axon();
    let dir = temp_dir("l1");
    let src = dir.join("orig.ax");
    let rec_a = dir.join("rec_a.json");
    let rec_b = dir.join("rec_b.json");
    let suite_a = dir.join("suite_a.ax");
    let suite_b = dir.join("suite_b.ax");
    std::fs::write(
        &src,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "identical-response-everywhere")
    let r = ask m { user: "q" }
    print(r)
}
"#,
    )
    .unwrap();
    // Record twice into different filenames, then promote both
    // without --name. The auto-derived names must differ because the
    // filename stems differ.
    for rec in [&rec_a, &rec_b] {
        let r = Command::new(axon_bin())
            .args(["run", "--record", rec.to_str().unwrap(), src.to_str().unwrap()])
            .output()
            .expect("axon run --record");
        assert!(r.status.success(), "record: {:?}", r);
    }
    for (rec, suite) in [(&rec_a, &suite_a), (&rec_b, &suite_b)] {
        let p = Command::new(axon_bin())
            .args([
                "trace",
                "promote",
                rec.to_str().unwrap(),
                "--to-suite",
                suite.to_str().unwrap(),
            ])
            .output()
            .expect("axon trace promote");
        assert!(
            p.status.success(),
            "promote: {}",
            String::from_utf8_lossy(&p.stderr)
        );
    }
    let body_a = std::fs::read_to_string(&suite_a).unwrap();
    let body_b = std::fs::read_to_string(&suite_b).unwrap();
    // Extract the `test "<name>"` line from each suite and confirm they
    // are not equal (the filename stem differentiates them).
    let name_line_a = body_a
        .lines()
        .find(|l| l.contains("test \""))
        .unwrap_or("");
    let name_line_b = body_b
        .lines()
        .find(|l| l.contains("test \""))
        .unwrap_or("");
    assert_ne!(
        name_line_a, name_line_b,
        "L1 regressed: identical content + different filenames \
         produced the same auto-name. a=`{name_line_a}` b=`{name_line_b}`"
    );
    assert!(name_line_a.contains("rec_a"), "name_a should embed filename stem: {name_line_a}");
    assert!(name_line_b.contains("rec_b"), "name_b should embed filename stem: {name_line_b}");
}

// =========================================================================
// S3 — escape_axon_string escapes bidi direction overrides and
// zero-width characters so a malicious recording can't hide injected
// content from a human reviewing the synthesized suite source.
// =========================================================================

#[test]
fn s3_trace_promote_escapes_bidi_and_zero_width_in_recorded_content() {
    build_axon();
    let dir = temp_dir("s3");
    let rec = dir.join("rec.json");
    let suite = dir.join("suite.ax");
    // Hand-craft a recording whose response content contains a bidi
    // direction override (U+202E, RLO) and a zero-width space (U+200B).
    // A naive synthesis would write them verbatim into the source,
    // hiding any visible content reversal from a human reviewer.
    let content = "hello\u{202E}reviewer\u{200B}injected";
    let raw = serde_json::json!({
        "version": 1,
        "events": [{
            "kind": "model_call",
            "provider": "mock",
            "response": {
                "content": content,
                "blocks": [],
                "structured": null,
                "tool_calls": [],
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "cached_input_tokens": 0,
                    "cost_usd": 0.0
                },
                "stop_reason": "end_turn"
            }
        }]
    });
    std::fs::write(&rec, raw.to_string()).unwrap();
    let out = Command::new(axon_bin())
        .args([
            "trace",
            "promote",
            rec.to_str().unwrap(),
            "--to-suite",
            suite.to_str().unwrap(),
            "--name",
            "s3check",
        ])
        .output()
        .expect("axon trace promote");
    assert!(
        out.status.success(),
        "promote: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = std::fs::read_to_string(&suite).unwrap();
    // No raw bidi/zero-width chars in the suite output — they must be
    // escaped to \u{202e} / \u{200b}.
    assert!(
        !body.contains('\u{202E}'),
        "S3 regressed: raw U+202E in suite body — would hide content from reviewers"
    );
    assert!(
        !body.contains('\u{200B}'),
        "S3 regressed: raw U+200B in suite body"
    );
    assert!(
        body.contains("\\u{202e}") || body.contains("\\u{202E}"),
        "expected U+202E to appear as `\\u{{202e}}` in body: {body}"
    );
}

// =========================================================================
// L2 — eval_parallel non-ask error message includes a workaround hint.
// =========================================================================

#[test]
fn l2_parallel_non_ask_error_mentions_workaround() {
    build_axon();
    let dir = temp_dir("l2");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model_slow("x", 1)
    parallel {
        ask m { user: "ok" },
        1 + 2,
    }
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Workaround") || stderr.contains("flow_parallel_asks"),
        "L2 regressed: error message should mention the workaround; got: {stderr}"
    );
}
