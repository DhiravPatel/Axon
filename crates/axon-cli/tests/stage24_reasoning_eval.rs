//! Stage 24 — §49 reasoning, §55 trajectory eval / redteam / sim,
//! §56 prefix cache end-to-end through the `axon` binary.

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
    p.push(format!("axon-stage24b-{name}-{pid}-{ts}"));
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

#[test]
fn reasoning_budget_debits_and_breaches() {
    build_axon();
    let dir = temp_dir("reasoning");
    let prog = r#"
fn main() uses { Console } {
    reasoning_budget_new("plan", "high", 1000, false)
    let breached_a = reasoning_budget_debit("plan", 400)
    print(bool(breached_a))
    let breached_b = reasoning_budget_debit("plan", 700)
    print(bool(breached_b))
    let s = reasoning_budget_status("plan")
    print(s.effort)
    print(bool(s.breached))
    print(s.spent)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false", "first 400 should not breach 1000 cap");
    assert_eq!(lines[1], "true", "1100 total > 1000 cap");
    assert_eq!(lines[2], "high");
    assert_eq!(lines[3], "true");
    assert_eq!(lines[4], "1100");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trajectory_metrics_compute_correctly() {
    build_axon();
    let dir = temp_dir("trajectory");
    let prog = r#"
fn main() uses { Console } {
    eval_trajectory_new(
        "t1",
        "find paper on X",
        list_new("search", "summarize"),
        list_new("shell")
    )
    eval_trajectory_add_step("t1", "first try", "search", "args", true, "")
    eval_trajectory_add_step("t1", "retry", "search", "args", false, "found paper Y")
    eval_trajectory_add_step("t1", "summarize", "summarize", "args", false, "summary")
    eval_trajectory_set_answer("t1", "Paper Y discusses X.")

    print(eval_trajectory_tool_accuracy("t1"))
    print(bool(eval_trajectory_recovered("t1")))
    print(bool(eval_trajectory_no_forbidden_tool("t1")))
    print(bool(eval_trajectory_no_secret_exposed("t1", list_new("CANARY"))))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // 2 good calls / 3 total = 0.666...
    assert!(lines[0].starts_with("0.66"), "got: {}", lines[0]);
    assert_eq!(lines[1], "true", "step1 errored, step2 succeeded");
    assert_eq!(lines[2], "true", "shell never called");
    assert_eq!(lines[3], "true", "canary never appears");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redteam_load_returns_known_suites() {
    build_axon();
    let dir = temp_dir("redteam");
    let prog = r#"
fn main() uses { Console } {
    let injection = redteam_load("std:injection")
    let unknown = redteam_load("not-a-real-suite")
    print(list_len(injection))
    print(list_len(unknown))
    let first = list_get(injection, 0)
    print(first.category)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].parse::<i64>().unwrap() >= 1);
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "prompt_injection");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sim_world_runs_until_agent_settles() {
    build_axon();
    let dir = temp_dir("sim");
    let prog = r#"
fn main() uses { Console } {
    sim_world_new("w", 42)
    sim_world_spawn("w", "buyer")
    sim_world_spawn("w", "seller")
    sim_world_script_send("w", "buyer", "seller", "offer:80")
    sim_world_script_note("w", "buyer", "wait", "considering")
    sim_world_script_settle("w", "buyer")
    sim_world_script_note("w", "seller", "wait", "thinking")
    sim_world_script_note("w", "seller", "wait", "counter")
    sim_world_script_settle("w", "seller")
    let hit = sim_world_run_until_settled("w", "buyer", 1000000000, 10)
    print(bool(hit))
    print(list_len(sim_world_events("w")))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert!(lines[1].parse::<i64>().unwrap() >= 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sim_rand_is_deterministic_for_same_seed() {
    build_axon();
    let dir = temp_dir("sim_rand");
    let prog = r#"
fn main() uses { Console } {
    sim_world_new("a", 123)
    sim_world_new("b", 123)
    print(sim_world_rand_u64("a"))
    print(sim_world_rand_u64("b"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], lines[1], "same seed -> same first draw");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cost_cache_hits_and_stats_work() {
    build_axon();
    let dir = temp_dir("cache");
    let prog = r#"
fn main() uses { Console } {
    cost_cache_clear()
    cost_cache_insert("system: be precise about citations", 800, 600)
    let a = cost_cache_lookup("system: be precise about citations")
    let b = cost_cache_lookup("system: be precise about citations")
    let miss = cost_cache_lookup("nothing-here")
    print(bool(a.hit))
    print(bool(b.hit))
    print(bool(miss.hit))
    let s = cost_cache_stats()
    print(s.hits)
    print(s.misses)
    print(s.tokens_saved)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], "false");
    assert_eq!(lines[3], "2");
    assert_eq!(lines[4], "1");
    assert_eq!(lines[5], "1600", "2 hits * 800 tokens");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn react_loop_drives_think_act_observe_until_done() {
    build_axon();
    let dir = temp_dir("react");
    let prog = r#"
fn think_step(log: dyn) -> String { "I should look this up" }
fn act_step(thought: dyn, log: dyn) -> String { "search('axon')" }
fn observe_step(thought: dyn, action: dyn) -> dyn {
    { observation: "found axon docs", done: true }
}
fn main() uses { Console } {
    let log = plan_react_loop(5, think_step, act_step, observe_step)
    print(list_len(log))
    let step0 = list_get(log, 0)
    print(step0.thought)
    print(step0.action)
    print(step0.observation)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "1", "stops after the first done=true");
    assert_eq!(lines[1], "I should look this up");
    assert_eq!(lines[2], "search('axon')");
    assert_eq!(lines[3], "found axon docs");
    let _ = std::fs::remove_dir_all(&dir);
}
