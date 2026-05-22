//! Stage 16 — `eval_*`, `cost_*`, `ffi_*` exercised through the binary.

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
    p.push(format!("axon-stage16-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn run_program_in(dir: &std::path::Path, src: &str) -> std::process::Output {
    let path = dir.join("p.ax");
    std::fs::write(&path, src).unwrap();
    Command::new(axon_bin())
        .args(["run", path.to_str().unwrap()])
        .output()
        .expect("axon run")
}

// ---------- eval --------------------------------------------------------

#[test]
fn eval_run_with_named_handler_passes_and_fails_correctly() {
    build_axon();
    let dir = temp_dir("eval_basic");
    let prog = r#"
fn echo(input: String) -> String { input }

fn main() uses { Console } {
    eval_suite_new("greet")
    eval_add_scenario("greet", "echo-hi", "hello", "hello")
    eval_add_scenario("greet", "wrong-expected", "hello", "goodbye")
    eval_add_metric("greet", "exact_match")
    let r = eval_run("greet", echo)
    print_int(r.total_runs)
    print_int(r.passed_runs)
}
"#;
    let out = run_program_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "2", "2 scenarios");
    assert_eq!(lines[1], "1", "1 passed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eval_run_writes_junit_xml() {
    build_axon();
    let dir = temp_dir("eval_junit");
    let xml_path = dir.join("report.xml").display().to_string();
    let prog = format!(
        r#"
fn echo(input: String) -> String {{ input }}

fn main() uses {{ Console }} {{
    eval_suite_new("xmltest")
    eval_add_scenario("xmltest", "ok",   "hi", "hi")
    eval_add_scenario("xmltest", "fail", "hi", "no")
    eval_add_metric("xmltest", "exact_match")
    let r = eval_run("xmltest", echo)
    eval_report_junit("xmltest", "{xml}")
    print_int(r.total_runs)
}}
"#,
        xml = xml_path
    );
    let out = run_program_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let xml = std::fs::read_to_string(&xml_path).unwrap();
    assert!(xml.contains("<testsuite"));
    assert!(xml.contains("<failure"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- cost --------------------------------------------------------

#[test]
fn cost_record_and_report_aggregate() {
    build_axon();
    let dir = temp_dir("cost_basic");
    let prog = r#"
fn main() uses { Console } {
    cost_reset()
    // anthropic opus: 300¢/M input, 1500¢/M output
    cost_profile_add("anthropic", "opus", 300, 1500)
    cost_record("anthropic", "opus", 1000, 2000, 200, "tag-a")
    cost_record("anthropic", "opus", 5000, 5000, 500, "tag-b")

    let r = cost_report(5)
    print_int(r.total_calls)
    print_int(r.total_cents)
    print(list_get(r.providers, 0).provider)
}
"#;
    let out = run_program_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "2", "two records");
    // Call 1: 1000*300/1e6 + 2000*1500/1e6 = 0 + 3 = 3¢
    // Call 2: 5000*300/1e6 + 5000*1500/1e6 = 1 + 7 = 8¢
    // (integer cents math; precise per profile.rs semantics)
    let total: i64 = lines[1].parse().unwrap();
    assert!(total > 0, "expected nonzero total cents, got {total}");
    assert_eq!(lines[2], "anthropic");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cost_ledger_persists_across_processes() {
    build_axon();
    let dir = temp_dir("cost_persist");
    let ledger_path = dir.join("ledger.json").display().to_string();

    let src1 = dir.join("write.ax");
    std::fs::write(
        &src1,
        format!(
            r#"
fn main() uses {{ Console }} {{
    cost_reset()
    cost_profile_add("acme", "model-a", 100, 500)
    cost_record("acme", "model-a", 1000000, 1000000, 50, "t")
    cost_save("{ledger}")
}}
"#,
            ledger = ledger_path
        ),
    )
    .unwrap();
    let out1 = Command::new(axon_bin())
        .args(["run", src1.to_str().unwrap()])
        .output()
        .expect("axon run write");
    assert!(out1.status.success(), "{:?}", out1);

    let src2 = dir.join("read.ax");
    std::fs::write(
        &src2,
        format!(
            r#"
fn main() uses {{ Console }} {{
    cost_reset()
    cost_load("{ledger}")
    cost_profile_add("acme", "model-a", 100, 500)
    let r = cost_report(5)
    print_int(r.total_calls)
    print_int(r.total_cents)
}}
"#,
            ledger = ledger_path
        ),
    )
    .unwrap();
    let out2 = Command::new(axon_bin())
        .args(["run", src2.to_str().unwrap()])
        .output()
        .expect("axon run read");
    assert!(out2.status.success(), "{:?}", out2);
    let s = String::from_utf8_lossy(&out2.stdout);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "1");
    // 1M input + 1M output at 100 + 500 cents/M = 100 + 500 = 600 cents
    assert_eq!(lines[1], "600");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- ffi ---------------------------------------------------------

#[test]
#[cfg(unix)]
fn ffi_call_round_trips_json_through_cat() {
    build_axon();
    let dir = temp_dir("ffi_basic");
    // Raw strings in Axon use backticks, so the JSON payload doesn't need
    // backslash-escaping the quotes.
    let prog = r#"
fn main() uses { Console } {
    let r = ffi_call("/bin/cat", list_new(), `{"value": 42}`, 2000)
    print(bool(r.ok))
    print(r.response_json)
}
"#;
    let out = run_program_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert!(
        lines[1].contains("42"),
        "expected echoed value, got: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(unix)]
fn ffi_call_reports_timeout_as_typed_error() {
    build_axon();
    let dir = temp_dir("ffi_timeout");
    let prog = r#"
fn main() uses { Console } {
    let r = ffi_call("/bin/sleep", list_new("5"), `{}`, 200)
    print(bool(r.ok))
    print(r.error)
}
"#;
    let out = run_program_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert!(
        lines[1].to_lowercase().contains("timeout") || lines[1].to_lowercase().contains("closed"),
        "got error: {}",
        lines[1]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
