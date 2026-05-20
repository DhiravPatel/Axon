//! Stage 24 — §29 multi-agent orchestration end-to-end through the
//! `axon` binary.

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
    p.push(format!("axon-stage24-{name}-{pid}-{ts}"));
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
fn network_cycle_detected() {
    build_axon();
    let dir = temp_dir("network_cycle");
    let prog = r#"
fn main() uses { Console } {
    flow_network_new("Team")
    flow_network_add_node("Team", "a")
    flow_network_add_node("Team", "b")
    flow_network_add_node("Team", "c")
    flow_network_add_edge("Team", "a", "b", "oneway")
    flow_network_add_edge("Team", "b", "c", "oneway")
    flow_network_add_edge("Team", "c", "a", "oneway")
    let r = flow_network_verify("Team")
    print(bool(r.ok))
    print(str_contains(r.error, "cycle"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert_eq!(lines[1], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn graph_topo_order_passed_in_dependency_order() {
    build_axon();
    let dir = temp_dir("graph_topo");
    let prog = r#"
fn main() uses { Console } {
    flow_graph_new("Triage")
    flow_graph_add_node("Triage", "classify", "fast")
    flow_graph_add_node("Triage", "retrieve", "kb")
    flow_graph_add_node("Triage", "draft", "brain")
    flow_graph_add_node("Triage", "review", "judge")
    flow_graph_add_edge("Triage", "classify", "draft")
    flow_graph_add_edge("Triage", "retrieve", "draft")
    flow_graph_add_edge("Triage", "draft", "review")
    let v = flow_graph_verify("Triage")
    print(bool(v.ok))
    let order = flow_graph_topo("Triage")
    print(list_get(order, 3))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    // The last node in topological order must be `review`.
    assert_eq!(lines[1], "review");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn debate_and_tot_round_trip() {
    build_axon();
    let dir = temp_dir("debate_tot");
    let prog = r#"
fn pro_arg(q: dyn, transcript: dyn) -> String { "pro-side" }
fn con_arg(q: dyn, transcript: dyn) -> String { "con-side" }
fn judge_arg(q: dyn, transcript: dyn) -> String { "pro wins" }

fn expand_thought(t: dyn, d: dyn) -> dyn {
    list_new(t * 2, t * 2 + 1, t + 5)
}
fn score_thought(t: dyn) -> Float { float(t) }

fn main() uses { Console } {
    let d = flow_debate("Should we ship?", pro_arg, con_arg, judge_arg, 2)
    print(d.verdict)
    print(list_len(d.transcript))

    let t = flow_tree_of_thought(1, expand_thought, score_thought, 2, 3)
    print(bool(t.best.thought >= 15))
    print(t.expansions)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "pro wins");
    assert_eq!(lines[1], "4"); // 2 rounds = 4 statements
    assert_eq!(lines[2], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn race_returns_first_accepted_candidate() {
    build_axon();
    let dir = temp_dir("race");
    let prog = r#"
fn cheap_model(q: String) -> String { "cheap:answer" }
fn pricey_model(q: String) -> String { "deep:answer" }
fn accept_cheap(s: String) -> Bool { str_starts_with(s, "cheap") }

fn main() uses { Console } {
    let r = flow_race("question", list_new(cheap_model, pricey_model), accept_cheap)
    print(r.winner_index)
    print(r.considered)
    print(bool(r.accepted))
    print(r.value)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "cheap:answer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_runs_step_over_each_input() {
    build_axon();
    let dir = temp_dir("batch");
    let prog = r#"
fn upper(s: String) -> String { str_upper(s) }

fn main() uses { Console } {
    let r = flow_batch(upper, list_new("a", "b", "c"))
    print(list_get(r, 0))
    print(list_get(r, 1))
    print(list_get(r, 2))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "A");
    assert_eq!(lines[1], "B");
    assert_eq!(lines[2], "C");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn difficulty_routes_to_correct_tier() {
    build_axon();
    let dir = temp_dir("difficulty");
    let prog = r#"
fn fast_model(q: String) -> String { "fast:reply" }
fn medium_model(q: String) -> String { "med:reply" }
fn deep_model(q: String) -> String { "deep:reply" }

fn main() uses { Console } {
    print(flow_estimate_difficulty("Hi"))
    print(flow_estimate_difficulty("Prove that 2+2=4 step by step"))
    let r = flow_route_difficulty("Prove", fast_model, medium_model, deep_model)
    print(r.tier)
    print(r.value)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "trivial");
    assert_eq!(lines[1], "hard");
    assert_eq!(lines[2], "hard");
    assert_eq!(lines[3], "deep:reply");
    let _ = std::fs::remove_dir_all(&dir);
}
