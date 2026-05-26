//! Stage 25 — end-to-end smoke tests for the new host bindings:
//! context policy, saga, durable timers, RAG grounding, media generation,
//! skill_use, agent card auto-publish, metrics, serverless render.

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
    p.push(format!("axon-stage25-{name}-{pid}-{ts}"));
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
fn timer_arm_due_mark_round_trip() {
    build_axon();
    let dir = temp_dir("timer");
    let prog = r#"
fn main() uses { Console } {
    timer_arm("t1", "wake", 1000, "payload-a")
    timer_arm("t2", "later", 9999999999999, "payload-b")
    print(timer_pending_count())
    let due = timer_due(5000)
    print(list_len(due))
    print(list_get(due, 0))
    timer_mark_fired("t1")
    print(timer_pending_count())
    print(list_len(timer_due(5000)))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "t1");
    assert_eq!(lines[3], "1");
    assert_eq!(lines[4], "0", "no more due after mark_fired");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saga_compensates_in_reverse_on_failure() {
    build_axon();
    let dir = temp_dir("saga");
    let prog = r#"
fn act_a(input: dyn) -> String { "seat-42" }
fn comp_a(value: dyn) -> dyn uses { Console } { print("released:" + str(value)) }
fn act_b(input: dyn) -> String { "payment-99" }
fn comp_b(value: dyn) -> dyn uses { Console } { print("refunded:" + str(value)) }
fn act_c_fails(input: dyn) -> String { panic("printer offline") }

fn main() uses { Console } {
    let actions = list_new(
        { action: act_a, compensate: comp_a },
        { action: act_b, compensate: comp_b },
        { action: act_c_fails, compensate: nil }
    )
    let r = flow_saga_run(0, list_new("reserve", "charge", "issue"), actions)
    print(r.status)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Compensations run LIFO: refund first, then release.
    assert_eq!(lines[0], "refunded:payment-99");
    assert_eq!(lines[1], "released:seat-42");
    assert_eq!(lines[2], "compensated");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rag_grounding_reports_ungrounded_claim() {
    build_axon();
    let dir = temp_dir("rag_ground");
    let prog = r#"
fn main() uses { Console } {
    let p1 = { id: "p1", text: "Cats sleep most of the day.", source: "", url: "" }
    let answer = "Cats sleep most of the day. Whales recite poetry under water."
    let cfg = { min_overlap: 0.6, grounded_threshold: 0.8, citation_threshold: 1.0 }
    let r = rag_assess_grounding(answer, list_new(p1), list_new(), cfg)
    print(r.grounded_fraction)
    print(bool(r.passed))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "0.5");
    assert_eq!(lines[1], "false");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn media_image_generation_returns_png_signature() {
    build_axon();
    let dir = temp_dir("media_image");
    let prog = r#"
fn main() uses { Console } {
    let r = media_generate_image("a happy cat", 256, 256, "png", 7, 1)
    let first = list_get(r, 0)
    print(first.format)
    print(first.width)
    print(first.bytes_len)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "png");
    assert_eq!(lines[1], "256");
    assert_eq!(lines[2], "8", "PNG signature is 8 bytes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skill_bind_reports_missing_caps() {
    build_axon();
    let dir = temp_dir("skill");
    let manifest_json = r#"{"name":"scraper","version":"0.1.0","entrypoint":"src/lib.ax","capabilities":["Net","Fs.Write"]}"#;
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let m = `{json}`
    let b = skill_bind(m, list_new("Net"), "")
    print(bool(b.is_satisfied))
    print(b.error)
}}
"#,
        json = manifest_json
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "false");
    assert!(lines[1].contains("Fs.Write"), "got: {}", lines[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn agent_card_derive_returns_valid_well_known_json() {
    build_axon();
    let dir = temp_dir("auto_card");
    let summary_json = r#"{
        "name": "Research",
        "version": "1.0.0",
        "description": "demo",
        "handlers": [
            {"name": "Research", "description": "literature search"}
        ],
        "schemas": {}
    }"#;
    let prog = format!(
        r#"
fn main() uses {{ Console }} {{
    let s = `{json}`
    let card_json = agent_card_derive(s, "https://research.example.com")
    print(str_contains(card_json, "research.example.com/agent"))
    print(str_contains(card_json, "Research"))
    print(agent_card_well_known_path())
}}
"#,
        json = summary_json
    );
    let out = run_in(&dir, &prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "true");
    assert_eq!(lines[2], ".well-known/agent-card.json");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metrics_record_then_render_emits_prometheus() {
    build_axon();
    let dir = temp_dir("metrics");
    let prog = r#"
fn main() uses { Console } {
    metrics_record(200, 100, 200, 500)
    metrics_record(500, 50, 80, 100)
    let body = metrics_render_prometheus()
    print(str_contains(body, "axon_requests_total 2"))
    print(str_contains(body, "axon_requests_success_total 1"))
    print(str_contains(body, "axon_requests_error_total 1"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    for l in &lines[..3] {
        assert_eq!(*l, "true", "got: {l}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serverless_render_emits_lambda_template() {
    build_axon();
    let dir = temp_dir("serverless");
    let prog = r#"
fn main() uses { Console } {
    let y = serverless_render("lambda", "main", "research")
    print(str_contains(y, "research.axskill"))
    print(str_contains(y, "AxonHandler: main"))
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "true");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn context_policy_drop_oldest_protects_system_and_last_turn() {
    build_axon();
    let dir = temp_dir("ctx_policy");
    let prog = r#"
fn main() uses { Console } {
    let policy = { on_overflow: { kind: "drop_oldest" }, max_tokens: 20, reserved_for_response: 5 }
    let m0 = { role: "system", text: str_repeat("s", 80), tokens: 0, seq: 0, relevance: 0.0 }
    let m1 = { role: "user", text: str_repeat("u", 80), tokens: 0, seq: 1, relevance: 0.0 }
    let m2 = { role: "assistant", text: str_repeat("a", 80), tokens: 0, seq: 2, relevance: 0.0 }
    let m3 = { role: "user", text: "qqqqq", tokens: 0, seq: 3, relevance: 0.0 }
    let msgs = list_new(m0, m1, m2, m3)
    let outcome = context_policy_plan(policy, msgs)
    print(outcome.action)
    print(list_len(outcome.kept))
    print(list_len(outcome.removed))
}
"#;
    let out = run_in(&dir, prog);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "drop_oldest");
    let kept: i64 = lines[1].parse().unwrap();
    assert!(kept >= 2, "expected at least system+last_turn kept, got {kept}");
    let _ = std::fs::remove_dir_all(&dir);
}
