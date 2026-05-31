//! Stage 35.2 — `axon watch <file>` live trace inspector.

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
    p.push(format!("axon-stage35watch-{name}-{}-{ts}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(dir: &std::path::Path, src: &str) -> PathBuf {
    let p = dir.join("p.ax");
    std::fs::write(&p, src).unwrap();
    p
}

#[test]
fn watch_runs_program_and_streams_at_least_one_span_per_ask() {
    build_axon();
    let dir = temp_dir("basic");
    // The runtime opens an `ask` span per LLM call. Mock model means
    // zero ms — we just need the span to fire.
    let path = write(
        &dir,
        r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "hi")
    let a = ask m { user: "q" }
    print(a)
}
"#,
    );
    let out = Command::new(axon_bin())
        .args(["watch", "--no-color", path.to_str().unwrap()])
        .output()
        .expect("axon watch");
    assert!(out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("axon watch: tracing"),
        "expected startup banner: {stderr}"
    );
    assert!(
        stderr.contains("ask"),
        "expected ask span in stream: {stderr}"
    );
}

#[test]
fn watch_stdout_carries_program_output_separately_from_trace_stream() {
    build_axon();
    let dir = temp_dir("split_streams");
    let path = write(
        &dir,
        r#"fn main() uses { Console } { print("hello-stdout") }
"#,
    );
    let out = Command::new(axon_bin())
        .args(["watch", "--no-color", path.to_str().unwrap()])
        .output()
        .expect("axon watch");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("hello-stdout"),
        "program output must be on stdout: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stdout.contains("axon watch: tracing"),
        "watch banner must NOT be on stdout: {stdout}"
    );
}

#[test]
fn watch_trace_flag_writes_jsonl_at_end_of_run() {
    build_axon();
    let dir = temp_dir("trace_flag");
    let path = write(
        &dir,
        r#"fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "x")
    let _ = ask m { user: "y" }
    print("done")
}
"#,
    );
    let trace_path = dir.join("t.jsonl");
    let out = Command::new(axon_bin())
        .args([
            "watch",
            "--no-color",
            "--trace",
            trace_path.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .expect("axon watch --trace");
    assert!(out.status.success(), "{:?}", out);
    assert!(trace_path.exists(), "trace file should be written");
    let body = std::fs::read_to_string(&trace_path).unwrap();
    // JSONL: at least one line, each parseable as JSON.
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "trace JSONL should have ≥1 line: {body}");
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
        assert!(v.get("name").is_some(), "span missing name field: {line}");
    }
}

#[test]
fn watch_help_lists_flags() {
    build_axon();
    let out = Command::new(axon_bin())
        .args(["watch", "--help"])
        .output()
        .expect("axon watch --help");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("usage: axon watch"), "{stdout}");
    assert!(stdout.contains("--trace"), "{stdout}");
    assert!(stdout.contains("--no-color"), "{stdout}");
}

#[test]
fn watch_unknown_flag_errors_cleanly() {
    build_axon();
    let dir = temp_dir("unknown_flag");
    let path = write(&dir, "fn main() uses { Console } { print(\"x\") }\n");
    let out = Command::new(axon_bin())
        .args(["watch", "--bogus", path.to_str().unwrap()])
        .output()
        .expect("axon watch --bogus");
    assert!(!out.status.success(), "expected refusal");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown flag"),
        "expected unknown-flag error: {stderr}"
    );
}
