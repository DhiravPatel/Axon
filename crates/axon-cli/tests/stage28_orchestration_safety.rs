//! Stage 28 — consensus + spawn_pool (§29.5), human pseudo-agent (§29.9),
//! policy block (§30), FFI bridges (§35.2), protocol adapters (§35.3).

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
    p.push(format!("axon-stage28-{name}-{pid}-{ts}"));
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

// ---------- §29.5 consensus + spawn_pool ----------

#[test]
fn consensus_majority_returns_winning_option() {
    build_axon();
    let dir = temp_dir("cons_majority");
    let prog = r#"
fn main() uses { Console } {
    let v1 = { voter: "a", choice: "ship", ranking: list_new(), confidence: 1.0 }
    let v2 = { voter: "b", choice: "ship", ranking: list_new(), confidence: 1.0 }
    let v3 = { voter: "c", choice: "wait", ranking: list_new(), confidence: 1.0 }
    let cfg = { rule: "majority", quorum_fraction: 0.0, expected_voters: 0, weights: { } }
    let d = flow_consensus(list_new(v1, v2, v3), cfg)
    print(d.outcome)
    print(d.vote_count)
    print(list_len(d.dissenting))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "ship");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn spawn_pool_builds_n_agents() {
    build_axon();
    let dir = temp_dir("spawn_pool");
    let prog = r#"
fn make_judge(i: Int) -> dyn {
    { judge_id: i, expertise: i * 10 }
}

fn main() uses { Console } {
    let pool = flow_spawn_pool(make_judge, 4)
    print(list_len(pool))
    print(list_get(pool, 0).judge_id)
    print(list_get(pool, 3).expertise)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "30");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §29.9 human pseudo-agent ----------

#[test]
fn human_request_round_trip() {
    build_axon();
    let dir = temp_dir("human_rt");
    let prog = r#"
fn main() uses { Console } {
    let id = human_request("slack:#treasury", "Approve $1.2k refund?", 600, "deny")
    let r = human_resolve(id)
    print(r.state)
    print(r.by)
    print(r.tool)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "pending");
    assert_eq!(lines[1], "slack:#treasury");
    assert_eq!(lines[2], "human:review");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_cancel_denies_pending_request() {
    build_axon();
    let dir = temp_dir("human_cancel");
    let prog = r#"
fn main() uses { Console } {
    let id = human_request("ch", "ok?", 60, "deny")
    print(human_cancel(id))
    print(human_resolve(id).state)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "denied");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §30 policy block ----------

#[test]
fn policy_block_allow_then_deny_default() {
    build_axon();
    let dir = temp_dir("policy_block");
    let prog = r#"
fn main() uses { Console } {
    policy_block_new("support", "deny")
    policy_block_allow("support", "tool", "kb.search", "")
    policy_block_allow("support", "tool", "kb.recall", "")

    let d1 = policy_block_check("support", "tool", "kb.search", true)
    print(bool(d1.allow))
    let d2 = policy_block_check("support", "tool", "payments.charge", true)
    print(bool(d2.allow))
    let summary = policy_block_audit_summary("support")
    print(summary.allow)
    print(summary.deny)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    assert_eq!(lines[2], "1");
    assert_eq!(lines[3], "1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn policy_block_budget_exhaustion_denies() {
    build_axon();
    let dir = temp_dir("policy_budget");
    let prog = r#"
fn main() uses { Console } {
    policy_block_new("budgeted", "allow")
    policy_block_add_budget("budgeted", "per_request", 50, -1)
    let d1 = policy_block_check("budgeted", "llm", "claude", true)
    print(bool(d1.allow))
    policy_block_charge("budgeted", "per_request", 50, 0)
    let d2 = policy_block_check("budgeted", "llm", "claude", true)
    print(bool(d2.allow))
    print(d2.label)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    assert_eq!(lines[2], "budget_exceeded");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn policy_block_when_clause_gates_allow() {
    build_axon();
    let dir = temp_dir("policy_when");
    let prog = r#"
fn main() uses { Console } {
    policy_block_new("refund", "deny")
    policy_block_allow("refund", "tool", "issue_refund", "amount <= 50")
    // When condition holds → allow.
    print(bool(policy_block_check("refund", "tool", "issue_refund", true).allow))
    // When condition fails → fall through to default-deny.
    print(bool(policy_block_check("refund", "tool", "issue_refund", false).allow))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §35.2 FFI bridges ----------

#[test]
fn ffi_bridge_call_rejects_unknown_kind() {
    build_axon();
    let dir = temp_dir("bridge_unknown");
    let prog = r#"
fn main() uses { Console } {
    let payload = `{}`
    let r = ffi_bridge_call("rustlang", "tools/x.rs", "main", payload, 1000)
    print(bool(r.ok))
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "kind validation should fail at host");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("python|node|wasm|grpc"),
        "got stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_bridge_call_to_nonexistent_python_fails_cleanly() {
    build_axon();
    let dir = temp_dir("bridge_missing");
    let prog = r#"
fn main() uses { Console } {
    let payload = `{"x":1}`
    let r = ffi_bridge_call("python", "/nonexistent/script.py", "main", payload, 200)
    print(bool(r.ok))
}
"#;
    let out = run_in(&dir, prog);
    // The Axon program runs (and prints `false`) even though the bridge
    // call failed — the call returns `{ok: false, error: ...}`.
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("false"), "expected ok=false: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- §35.3 protocol adapters ----------

#[test]
fn protocol_mcp_tools_list_replies_immediately() {
    build_axon();
    let dir = temp_dir("proto_mcp");
    let prog = r#"
fn main() uses { Console } {
    let body = `{"jsonrpc":"2.0","method":"tools/list","id":1}`
    let action = serve_protocol_route("mcp", "POST", "/", body, "main")
    print(action.kind)
    print(action.status)
    print(str_contains(action.body, "tools"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "reply");
    assert_eq!(lines[1], "200");
    assert_eq!(lines[2], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn protocol_openai_dispatches_with_translated_prompt() {
    build_axon();
    let dir = temp_dir("proto_openai");
    let prog = r#"
fn main() uses { Console } {
    let body = `{"messages":[{"role":"system","content":"be brief"},{"role":"user","content":"hi"}]}`
    let action = serve_protocol_route("openai", "POST", "/v1/chat/completions", body, "main")
    print(action.kind)
    print(action.handler)
    print(str_contains(action.prompt, "be brief"))
    print(str_contains(action.prompt, "user: hi"))

    let wrapped = serve_protocol_wrap("openai", "the answer", "null")
    print(wrapped.status)
    print(str_contains(wrapped.body, "the answer"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "dispatch");
    assert_eq!(lines[1], "main");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "true");
    assert_eq!(lines[4], "200");
    assert_eq!(lines[5], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn protocol_a2a_well_known_returns_card() {
    build_axon();
    let dir = temp_dir("proto_a2a");
    let prog = r#"
fn main() uses { Console } {
    let action = serve_protocol_route("a2a", "GET", "/.well-known/agent-card.json", "", "main")
    print(action.kind)
    print(action.status)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "reply");
    assert_eq!(lines[1], "200");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn render_grpc_proto_emits_service_block() {
    build_axon();
    let dir = temp_dir("grpc_proto");
    let prog = r#"
fn main() uses { Console } {
    let body = serve_render_grpc_proto("Support", list_new("Triage", "Resolve"))
    print(str_contains(body, "service Support"))
    print(str_contains(body, "rpc Triage"))
    print(str_contains(body, "rpc Resolve"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines().take(3) {
        assert_eq!(line, "true");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serve_protocol_flag_validates_value() {
    build_axon();
    let out = Command::new(axon_bin())
        .args(["serve", "--protocol", "websocket", "x.ax"])
        .output()
        .expect("axon serve");
    assert!(!out.status.success(), "should reject unknown protocol");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("plain|mcp|openai|grpc|a2a"),
        "stderr: {stderr}"
    );
}
