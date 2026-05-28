//! Stage 30 — native syntax (try/recover, ??, ?., it, policy block,
//! extern bridges) + developer-experience CLI (explain, --json,
//! new, stats, doctor, completions, run --dry-run).

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
    p.push(format!("axon-stage30-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
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

// ---------- native syntax ----------

#[test]
fn try_recover_runs_and_recovers() {
    build_axon();
    let dir = temp_dir("tryrec");
    let prog = r#"
fn risky(n: Int) -> Int { if n == 0 { panic("zero") } else { 100 / n } }
fn main() uses { Console } {
    print_int(try { risky(4) } recover |e| { -1 })
    print_int(try { risky(0) } recover |e| { -1 })
    print(try { risky(0) } recover |e| { e })
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "25");
    assert_eq!(lines[1], "-1");
    // `panic("zero")` surfaces as the message "panic: zero" to the
    // recover binding.
    assert_eq!(lines[2], "panic: zero");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coalesce_and_safe_access() {
    build_axon();
    let dir = temp_dir("coalesce");
    let prog = r#"
fn main() uses { Console } {
    print_int(opt_none() ?? 42)
    print_int(opt_some(7) ?? 99)
    let rec = { name: "Sam", city: "Paris" }
    print(rec?.city)
    print(bool(opt_is_none(opt_none()?.city)))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "7");
    assert_eq!(lines[2], "Paris");
    assert_eq!(lines[3], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_closure_and_thunk_coexist() {
    build_axon();
    let dir = temp_dir("it");
    let prog = r#"
fn apply(f: dyn, x: Int) -> Int { f(x) }
fn main() uses { Console } {
    print_int(apply(|| it * 2, 21))
    let thunk = || 7
    print_int(thunk())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "7");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn policy_block_native_syntax_enforced() {
    build_axon();
    let dir = temp_dir("policy");
    let prog = r#"
policy support {
    allow tool kb.search, tickets.get
    allow tool issue_refund when amount <= 50
    deny tool payments.charge
    deny net "*"
    budget per_request { usd = 0.50, tokens = 60000 }
    rate per_user { 30 per 1m }
    audit all_tool_calls
}

fn main() uses { Console } {
    print(bool(policy_block_check("support", "tool", "kb.search", true).allow))
    print(bool(policy_block_check("support", "tool", "payments.charge", true).allow))
    print(bool(policy_block_check("support", "tool", "issue_refund", false).allow))
    print(bool(policy_block_check("support", "net", "evil.com", true).allow))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines, vec!["true", "false", "false", "false"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extern_python_tool_bridge() {
    build_axon();
    if !which("python3") {
        return; // skip on hosts without python3
    }
    let dir = temp_dir("extern");
    std::fs::write(
        dir.join("s.py"),
        "import sys, json\ndef run(a):\n    return {\"ok\": True, \"value\": (0.9 if \"love\" in a.get(\"text\",\"\").lower() else 0.1)}\nif __name__ == \"__main__\":\n    line = sys.stdin.readline()\n    print(json.dumps(run(json.loads(line) if line.strip() else {})))\n",
    )
    .unwrap();
    let prog = format!(
        r#"
tool sentiment(text: String) -> Float uses {{ Process }} extern python "{}/s.py:run"

fn main() uses {{ Console, Process }} {{
    print(sentiment("I love this"))
    print(sentiment("meh"))
}}
"#,
        dir.display()
    );
    let out = run_src(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(lines[0], "0.9");
    assert_eq!(lines[1], "0.1");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §57 diagnostics ----------

#[test]
fn explain_code_and_concept() {
    build_axon();
    let out = Command::new(axon_bin()).args(["explain", "E0202"]).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("cannot find name in scope"));

    let out = Command::new(axon_bin()).args(["explain", "effect:LLM"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("language model"));
}

#[test]
fn check_json_diagnostics() {
    build_axon();
    let dir = temp_dir("json");
    let path = dir.join("bad.ax");
    std::fs::write(&path, "fn main() uses { Console } {\n  print(nope)\n}\n").unwrap();
    let out = Command::new(axon_bin())
        .args(["check", "--json", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("\"code\": \"E0202\""), "got: {s}");
    assert!(s.contains("\"ok\": false"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_explain_errors_inlines_help() {
    build_axon();
    let dir = temp_dir("explainerr");
    let path = dir.join("bad.ax");
    std::fs::write(&path, "fn main() uses { Console } {\n  print(nope)\n}\n").unwrap();
    let out = Command::new(axon_bin())
        .args(["check", "--explain-errors", path.to_str().unwrap()])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stderr);
    assert!(s.contains("explain E0202"), "got: {s}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §58 onboarding ----------

#[test]
fn new_scaffolds_and_runs_every_template() {
    build_axon();
    let base = temp_dir("new");
    for t in [
        "agent", "support", "research", "assistant", "pipeline", "webhook",
        "lambda", "skill",
    ] {
        let name = format!("p_{t}");
        let out = Command::new(axon_bin())
            .args(["new", &name, "--template", t])
            .current_dir(&base)
            .output()
            .unwrap();
        assert!(out.status.success(), "new {t}: {:?}", out);
        let proj = base.join(&name);
        let run = Command::new(axon_bin())
            .args(["run"])
            .current_dir(&proj)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "template {t} failed to run: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn tour_lists_lessons() {
    build_axon();
    let out = Command::new(axon_bin()).args(["tour"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("lessons"));
}

// ---------- §64.2 QoL ----------

#[test]
fn stats_counts_a_project() {
    build_axon();
    let base = temp_dir("stats");
    Command::new(axon_bin())
        .args(["new", "sp", "-t", "research"])
        .current_dir(&base)
        .output()
        .unwrap();
    let out = Command::new(axon_bin())
        .args(["stats"])
        .current_dir(base.join("sp"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("agents       1"), "got: {s}");
    assert!(s.contains("tools        1"), "got: {s}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn run_dry_run_does_not_execute() {
    build_axon();
    let dir = temp_dir("dry");
    let path = dir.join("p.ax");
    std::fs::write(
        &path,
        "model brain = mock_model(\"fixed\", \"x\")\nfn main() uses { Console } { print(\"SHOULD NOT PRINT\") }\n",
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", "--dry-run", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("nothing was executed"));
    assert!(s.contains("models       brain"));
    assert!(!s.contains("SHOULD NOT PRINT"), "dry-run must not execute main");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn completions_emit_for_shells() {
    build_axon();
    for sh in ["bash", "zsh", "fish", "pwsh"] {
        let out = Command::new(axon_bin())
            .args(["completions", sh])
            .output()
            .unwrap();
        assert!(out.status.success(), "completions {sh}");
        assert!(!String::from_utf8_lossy(&out.stdout).is_empty());
    }
}

#[test]
fn doctor_runs() {
    build_axon();
    let out = Command::new(axon_bin()).args(["doctor"]).output().unwrap();
    // Exit code may be 0 or 1 depending on environment; output is what matters.
    assert!(String::from_utf8_lossy(&out.stdout).contains("axon doctor"));
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}
