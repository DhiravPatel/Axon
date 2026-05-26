//! Stage 29 — Result<T,E> + try_recover (§19), Stream<T> + for_await (§28),
//! @restart variant validation (§29.7), `axon prof --cost` (§31.2).

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
    p.push(format!("axon-stage29-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_in(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

// ---------- §19 Result type + try_recover ----------

#[test]
fn result_type_annotation_parses() {
    build_axon();
    let dir = temp_dir("result_type");
    let prog = r#"
fn divide(a: Int, b: Int) -> Result<Int, String> {
    if b == 0 { result_err("divide by zero") } else { result_ok(a / b) }
}

fn main() uses { Console } {
    let r1 = divide(10, 2)
    print(bool(result_is_ok(r1)))
    print(result_value(r1))
    let r2 = divide(7, 0)
    print(bool(result_is_err(r2)))
    print(result_error(r2))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "divide by zero");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn try_recover_calls_recovery_on_error() {
    build_axon();
    let dir = temp_dir("try_recover");
    let prog = r#"
fn fragile() -> dyn { panic("boom") }
fn safe() -> Int { 42 }
fn fallback(msg: dyn) -> Int { 99 }

fn main() uses { Console } {
    let v = try_recover(fragile, fallback)
    print(v)
    let ok = try_recover(safe, fallback)
    print(ok)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "99", "fragile() panic -> fallback");
    assert_eq!(lines[1], "42", "safe() success -> passed through");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §28 Stream<T> + for_await ----------

#[test]
fn stream_send_take_round_trip() {
    build_axon();
    let dir = temp_dir("stream_rt");
    let prog = r#"
fn main() uses { Console } {
    stream_new("events", 4, "block")
    print(stream_send("events", "evt-1"))
    print(stream_send("events", "evt-2"))
    let a = stream_take("events")
    print(a.value)
    let b = stream_take("events")
    print(b.value)
    let c = stream_take("events")
    print(bool(c.has_value))
    stream_close("events")
    print(bool(stream_is_done("events")))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "buffered");
    assert_eq!(lines[1], "buffered");
    assert_eq!(lines[2], "evt-1");
    assert_eq!(lines[3], "evt-2");
    assert_eq!(lines[4], "false");
    assert_eq!(lines[5], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stream_backpressure_policy_block() {
    build_axon();
    let dir = temp_dir("stream_bp");
    let prog = r#"
fn main() uses { Console } {
    stream_new("s", 2, "block")
    stream_send("s", 1)
    stream_send("s", 2)
    print(stream_send("s", 3))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "backpressure");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stream_drop_oldest_keeps_newest() {
    build_axon();
    let dir = temp_dir("stream_drop_oldest");
    let prog = r#"
fn main() uses { Console } {
    stream_new("s", 2, "drop_oldest")
    stream_send("s", 1)
    stream_send("s", 2)
    print(stream_send("s", 3))
    let stats = stream_stats("s")
    print(stats.dropped)
    let a = stream_take("s")
    let b = stream_take("s")
    print(a.value)
    print(b.value)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "dropped_oldest");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "2");
    assert_eq!(lines[3], "3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn for_await_drains_stream_until_closed() {
    build_axon();
    let dir = temp_dir("for_await");
    let prog = r#"
fn collect(v: dyn) -> dyn uses { Console } { print(v) }

fn main() uses { Console } {
    stream_new("nums", 4, "block")
    stream_send("nums", 10)
    stream_send("nums", 20)
    stream_send("nums", 30)
    stream_close("nums")
    let n = for_await("nums", collect)
    print("drained: ")
    print(n)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
    assert_eq!(lines[3], "drained: ");
    assert_eq!(lines[4], "3");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §29.7 @restart variants ----------

#[test]
fn restart_policy_parse_accepts_three_variants() {
    build_axon();
    let dir = temp_dir("restart_parse");
    let prog = r#"
fn main() uses { Console } {
    print(restart_policy_parse("Permanent"))
    print(restart_policy_parse("Transient"))
    print(restart_policy_parse("Temporary"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "Permanent");
    assert_eq!(lines[1], "Transient");
    assert_eq!(lines[2], "Temporary");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restart_policy_rejects_unknown_variant() {
    build_axon();
    let dir = temp_dir("restart_bad");
    let prog = r#"
fn main() {
    restart_policy_parse("Forever")
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should reject unknown variant");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Permanent | Transient | Temporary"),
        "stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restart_policy_should_restart_decision_table() {
    build_axon();
    let dir = temp_dir("restart_decisions");
    let prog = r#"
fn main() uses { Console } {
    print(bool(restart_policy_should_restart("Permanent", "normal")))
    print(bool(restart_policy_should_restart("Permanent", "abnormal")))
    print(bool(restart_policy_should_restart("Transient", "normal")))
    print(bool(restart_policy_should_restart("Transient", "abnormal")))
    print(bool(restart_policy_should_restart("Temporary", "normal")))
    print(bool(restart_policy_should_restart("Temporary", "abnormal")))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["true", "true", "false", "true", "false", "false"]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §31.2 axon prof --cost ----------

#[test]
fn axon_prof_renders_cost_report() {
    build_axon();
    let dir = temp_dir("prof_cost");
    let ledger = dir.join("ledger.json");
    // Hand-craft a ledger in the shape `Ledger { entries: [CostEntry] }`.
    std::fs::write(
        &ledger,
        r#"{
            "entries": [
                {"provider":"anthropic","model":"opus","input_tokens":1000,"output_tokens":2000,"cached_input_tokens":0,"latency_ms":1200,"timestamp_ns":0,"tag":"agent.research"},
                {"provider":"anthropic","model":"opus","input_tokens":5000,"output_tokens":5000,"cached_input_tokens":0,"latency_ms":2400,"timestamp_ns":1,"tag":"agent.research"},
                {"provider":"openai","model":"gpt-4","input_tokens":2000,"output_tokens":500,"cached_input_tokens":0,"latency_ms":800,"timestamp_ns":2,"tag":"agent.qa"}
            ]
        }"#,
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args([
            "prof",
            "--cost",
            ledger.to_str().unwrap(),
            "--profile",
            "anthropic:300/1500",
            "--profile",
            "openai:200/1200",
            "--top",
            "5",
        ])
        .output()
        .expect("axon prof");
    assert!(out.status.success(), "stderr: {}",
        String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("total calls : 3"), "stdout: {stdout}");
    assert!(stdout.contains("anthropic"), "stdout: {stdout}");
    assert!(stdout.contains("openai"), "stdout: {stdout}");
    assert!(stdout.contains("per-provider breakdown"), "stdout: {stdout}");
    assert!(stdout.contains("top-3 most expensive calls"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn axon_prof_without_cost_flag_errors() {
    build_axon();
    let out = Command::new(axon_bin())
        .args(["prof"])
        .output()
        .expect("axon prof");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("usage: axon prof"), "stderr: {stderr}");
}

#[test]
fn axon_prof_rejects_bad_profile_spec() {
    build_axon();
    let dir = temp_dir("prof_bad_profile");
    let ledger = dir.join("ledger.json");
    std::fs::write(&ledger, r#"{"entries":[]}"#).unwrap();
    let out = Command::new(axon_bin())
        .args([
            "prof",
            "--cost",
            ledger.to_str().unwrap(),
            "--profile",
            "no-colon-here",
        ])
        .output()
        .expect("axon prof");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing `:`"), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
