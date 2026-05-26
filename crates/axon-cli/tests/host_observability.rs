//! Stage 20 — OTLP export + `axon replay/--patch` + `axon trace` + `axon repl`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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
    p.push(format!("axon-stage20-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

// ---------- OTLP export -------------------------------------------------

#[test]
fn trace_export_otlp_writes_a_collector_friendly_doc() {
    build_axon();
    let dir = temp_dir("otlp");
    let otlp_path = dir.join("traces.json").display().to_string();
    let trace_path = dir.join("internal.jsonl").display().to_string();

    // A tiny program that makes at least one model call so the runtime
    // produces an `Ask` span we can export. `mock_model` keeps it offline.
    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        format!(
            r#"
fn main() uses {{ Console, LLM, Net }} {{
    let m = mock_model("fixed", "hello back")
    let _ = ask m {{
        system: "test"
        user:   "hi"
    }} await
    trace_export_otlp("{otlp}", "demo-svc")
}}
"#,
            otlp = otlp_path
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", "--trace", &trace_path, src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "run: {:?}", out);

    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&otlp_path).unwrap()).unwrap();
    let rs = &doc["resourceSpans"];
    assert!(rs.is_array(), "missing resourceSpans");
    let inner = &rs[0]["scopeSpans"][0]["spans"];
    assert!(inner.is_array());
    assert!(!inner.as_array().unwrap().is_empty(), "no exported spans");
    // service.name picked up from the call.
    let svc = &rs[0]["resource"]["attributes"][0]["value"]["stringValue"];
    assert_eq!(svc.as_str().unwrap(), "demo-svc");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_export_otlp_errors_when_tracing_disabled() {
    build_axon();
    let dir = temp_dir("otlp_disabled");
    let path = dir.join("traces.json").display().to_string();
    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        format!(
            r#"
fn main() {{
    trace_export_otlp("{path}", "svc")
}}
"#,
            path = path
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(!out.status.success(), "should have failed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("tracing is not enabled"),
        "expected tracing-disabled error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- axon replay --------------------------------------------------

#[test]
fn axon_replay_uses_recorded_responses() {
    build_axon();
    let dir = temp_dir("replay_basic");
    let rec_path = dir.join("rec.json");
    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "from-recording")
    let r: String = ask m {
        system: "x"
        user:   "y"
    } await
    print(r)
}
"#,
    )
    .unwrap();

    // First: record.
    let rec_out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec_path.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(rec_out.status.success(), "record run: {:?}", rec_out);
    assert!(rec_path.exists(), "recording not written");

    // Second: replay.
    let replay_out = Command::new(axon_bin())
        .args([
            "replay",
            rec_path.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon replay");
    assert!(replay_out.status.success(), "replay: {:?}", replay_out);
    let stdout = String::from_utf8_lossy(&replay_out.stdout);
    assert!(
        stdout.contains("from-recording"),
        "expected the recorded text in stdout: {stdout:?}"
    );
    let stderr = String::from_utf8_lossy(&replay_out.stderr);
    assert!(
        stderr.contains("consumed 1 of 1 recorded event"),
        "expected replay progress in stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_replay_patch_tolerates_extra_program_calls() {
    build_axon();
    let dir = temp_dir("replay_patch");
    let rec_path = dir.join("rec.json");

    // Record a program that makes ONE model call.
    let src_one = dir.join("one_call.ax");
    std::fs::write(
        &src_one,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "first")
    let _ = ask m { user: "go" } await
}
"#,
    )
    .unwrap();
    let rec_out = Command::new(axon_bin())
        .args([
            "run",
            "--record",
            rec_path.to_str().unwrap(),
            src_one.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --record");
    assert!(rec_out.status.success(), "record: {:?}", rec_out);

    // Edit the program to add a second call. Strict replay should fail;
    // `--patch` should report the divergence cleanly.
    let src_two = dir.join("two_calls.ax");
    std::fs::write(
        &src_two,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "first")
    let _ = ask m { user: "go" } await
    let _ = ask m { user: "go-again" } await
}
"#,
    )
    .unwrap();

    // Strict (no --patch) → hard error.
    let strict = Command::new(axon_bin())
        .args([
            "replay",
            rec_path.to_str().unwrap(),
            src_two.to_str().unwrap(),
        ])
        .output()
        .expect("strict replay");
    assert!(!strict.status.success(), "strict replay should fail");
    let strict_err = String::from_utf8_lossy(&strict.stderr);
    assert!(
        strict_err.contains("replay exhausted"),
        "expected exhausted error: {strict_err}"
    );

    // Patch mode → still fails (we can't synthesize a response for the
    // extra call) but the error message mentions patch mode.
    let patched = Command::new(axon_bin())
        .args([
            "replay",
            rec_path.to_str().unwrap(),
            src_two.to_str().unwrap(),
            "--patch",
        ])
        .output()
        .expect("patch replay");
    let patch_err = String::from_utf8_lossy(&patched.stderr);
    assert!(
        patch_err.contains("patch mode") || patch_err.contains("[patch]"),
        "expected patch-mode signaling in stderr: {patch_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- axon trace pretty-printer -----------------------------------

#[test]
fn axon_trace_pretty_prints_a_jsonl_file() {
    build_axon();
    let dir = temp_dir("trace_pp");
    let trace_path = dir.join("trace.jsonl");
    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "ok")
    let _ = ask m { user: "hi" } await
}
"#,
    )
    .unwrap();

    // Generate a trace file.
    let run = Command::new(axon_bin())
        .args([
            "run",
            "--trace",
            trace_path.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .expect("axon run --trace");
    assert!(run.status.success(), "run: {:?}", run);
    assert!(trace_path.exists());

    // Pretty-print.
    let pp = Command::new(axon_bin())
        .args(["trace", trace_path.to_str().unwrap()])
        .output()
        .expect("axon trace");
    assert!(pp.status.success(), "trace pretty-print: {:?}", pp);
    let stdout = String::from_utf8_lossy(&pp.stdout);
    assert!(
        stdout.contains("trace: ") && stdout.contains("span"),
        "expected trace summary in output: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_trace_reports_empty_file_gracefully() {
    build_axon();
    let dir = temp_dir("trace_empty");
    let path = dir.join("empty.jsonl");
    std::fs::write(&path, b"").unwrap();
    let out = Command::new(axon_bin())
        .args(["trace", path.to_str().unwrap()])
        .output()
        .expect("axon trace");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("(no spans)"), "got: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- axon repl ---------------------------------------------------

#[test]
fn axon_repl_evaluates_input_and_quits_cleanly() {
    build_axon();
    let mut child = Command::new(axon_bin())
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn axon repl");
    {
        let stdin = child.stdin.as_mut().expect("stdin handle");
        // Two trivial statements that test arithmetic + the quit path.
        let _ = stdin.write_all(b"print_int(1 + 2)\n.quit\n");
    }
    let out = child.wait_with_output().expect("wait_with_output");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Axon ") && stdout.contains("REPL"),
        "expected banner in stdout: {stdout}"
    );
    assert!(
        stdout.contains('3'),
        "expected `3` from `print_int(1 + 2)`: {stdout}"
    );
}

#[test]
fn axon_repl_help_dot_command_prints_usage() {
    build_axon();
    let mut child = Command::new(axon_bin())
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn axon repl");
    {
        let stdin = child.stdin.as_mut().expect("stdin handle");
        let _ = stdin.write_all(b".help\n.quit\n");
    }
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(".help") && stdout.contains(".quit"),
        "expected help text: {stdout}"
    );
}
