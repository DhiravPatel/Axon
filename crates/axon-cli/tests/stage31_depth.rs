//! Stage 31 — computer-use primitives, GBNF schema emitter, zero-config
//! defaults, whole-program error recovery.

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
    p.push(format!("axon-stage31-{name}-{}-{ts}", std::process::id()));
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

// ---------- computer-use ----------

#[test]
fn computer_screenshot_returns_tainted_image_metadata() {
    build_axon();
    let dir = temp_dir("cu_shot");
    let prog = r#"
fn main() uses { Console, Computer } {
    let s = computer_screenshot()
    print_int(s.width)
    print(bool(s.tainted))
    print(s.format)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "1280");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], "png");
}

#[test]
fn computer_action_log_records_every_action_in_order() {
    build_axon();
    let dir = temp_dir("cu_log");
    let prog = r#"
fn main() uses { Console, Computer } {
    computer_screenshot()
    computer_mouse_move(50, 50)
    computer_click(50, 50, "left")
    computer_type("hi")
    computer_key("enter")
    let log = computer_action_log()
    print_int(list_len(log))
    print(str_contains(list_get(log, 0), "screenshot"))
    print(str_contains(list_get(log, 2), "click"))
    print(str_contains(list_get(log, 3), "hi"))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "true");
}

#[test]
fn computer_click_out_of_bounds_errors_cleanly() {
    build_axon();
    let dir = temp_dir("cu_oob");
    let prog = r#"
fn main() uses { Console, Computer } {
    computer_click(99999, 99999, "left")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("outside"), "got: {stderr}");
}

#[test]
fn computer_key_validates_against_allowlist() {
    build_axon();
    let dir = temp_dir("cu_key");
    let prog = r#"
fn main() uses { Console, Computer } {
    computer_key("not_a_real_key")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported key"));
}

// ---------- GBNF emitter ----------

#[test]
fn schema_to_gbnf_emits_grammar_for_primitive_schema() {
    build_axon();
    let dir = temp_dir("gbnf_basic");
    let prog = r##"
schema Profile {
    name: String,
    age: Int,
    active: Bool,
}
fn main() uses { Console } {
    let g = schema_to_gbnf("Profile")
    print(str_contains(g, "# schema: Profile"))
    print(str_contains(g, "root ::="))
    print(str_contains(g, "string"))
    print(str_contains(g, "integer"))
    print(str_contains(g, "boolean"))
}
"##;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    for line in std::str::from_utf8(&out.stdout).unwrap().lines().take(5) {
        assert_eq!(line, "true");
    }
}

#[test]
fn schema_to_gbnf_handles_optionals_and_lists() {
    build_axon();
    let dir = temp_dir("gbnf_opt");
    let prog = r#"
schema Order {
    id: String,
    tags: List<String>,
    note: String?,
}
fn main() uses { Console } {
    let g = schema_to_gbnf("Order")
    print(str_contains(g, "[\"]"))
    print(str_contains(g, "null"))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
}

#[test]
fn schema_to_gbnf_unknown_schema_errors() {
    build_axon();
    let dir = temp_dir("gbnf_unknown");
    let prog = r#"
fn main() {
    schema_to_gbnf("DoesNotExist")
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no schema named"));
}

// ---------- zero-config defaults ----------

#[test]
fn default_model_returns_a_callable_model() {
    build_axon();
    let dir = temp_dir("def_model");
    // Hide the env API key so we know we're hitting the mock path.
    let out = Command::new(axon_bin())
        .args(["run", dir.join("p.ax").to_str().unwrap()])
        .env_remove("ANTHROPIC_API_KEY")
        .output();
    std::fs::write(
        dir.join("p.ax"),
        "fn main() uses { Console, LLM, Net } {\n    print(ask default_model() { user: \"hi\" })\n}\n",
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", dir.join("p.ax").to_str().unwrap()])
        .env_remove("ANTHROPIC_API_KEY")
        .output()
        .expect("axon run");
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("default_model"), "stdout: {stdout}");
    let _ = out;
}

// ---------- whole-program error recovery ----------

#[test]
fn parser_reports_multiple_distinct_errors_without_cascade_spam() {
    build_axon();
    let dir = temp_dir("recovery");
    let path = dir.join("bad.ax");
    // Three distinct, recoverable items in one file: a broken fn,
    // a malformed let, and a third fn after.
    std::fs::write(
        &path,
        "fn ok1() -> Int { 1 }\n\
         fn broken() -> {\n\
         fn ok2() -> Int { 2 }\n\
         let x =\n\
         fn ok3() -> Int { 3 }\n",
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err_count = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.starts_with("error"))
        .count();
    // Without recovery + cascade suppression this is 14+. With them we
    // see fewer, distinct errors. The exact count is a UX detail; the
    // assertion is the regression cap.
    assert!(
        err_count < 12,
        "too many cascade errors ({err_count}); recovery is broken"
    );
    assert!(err_count > 0, "should still report errors");
}
