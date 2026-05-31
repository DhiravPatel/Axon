//! Stage 35.1 — native agent declaration slots.
//!
//! The parser already accepts arbitrary `key: value` Setting members
//! inside an agent block. Stage 35 makes four well-known keys
//! semantic slots that desugar at spawn time:
//!
//!   uses_tools: [...]  → self.tools     : List<dyn>
//!   memory: expr       → self.memory    : Memory
//!   policy: ident      → self.policy    : String  (= the ident's name)
//!   strategy: ident|str → self.strategy : String
//!
//! These tests drive the binary and inspect runtime output.

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
    p.push(format!("axon-stage35slots-{name}-{}-{ts}", std::process::id()));
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

#[test]
fn uses_tools_slot_exposes_self_dot_tools_as_a_list() {
    build_axon();
    let dir = temp_dir("uses_tools");
    let prog = r#"
tool a(x: String) -> String uses { Net } { x }
tool b(x: String) -> String uses { Net } { x }
tool c(x: String) -> String uses { Net } { x }

agent T() {
    uses_tools: [a, b, c],

    on count() -> Int uses { Tool } {
        list_len(self.tools)
    }
}

fn main() uses { Spawn, Tool, Console } {
    let t = spawn T()
    print_int(t.count())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_end().ends_with("3"), "stdout: {stdout}");
}

#[test]
fn memory_slot_exposes_self_dot_memory_with_full_recall_round_trip() {
    build_axon();
    let dir = temp_dir("memory");
    let prog = r#"
agent T() {
    memory: local_memory(),

    on note(s: String) -> Int uses { Memory } {
        self.memory.store(s)
        list_len(self.memory.recall(s))
    }
}

fn main() uses { Spawn, Memory, Console } {
    let t = spawn T()
    print_int(t.note("first"))
    print_int(t.note("second"))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // First store+recall: at least 1 hit; second: at least 1 hit (and
    // possibly more if "second" matches earlier entries).
    assert!(lines.len() >= 2, "{stdout}");
    let first: i32 = lines[0].parse().unwrap_or(0);
    let second: i32 = lines[1].parse().unwrap_or(0);
    assert!(first >= 1, "first recall returned {first}: {stdout}");
    assert!(second >= 1, "second recall returned {second}: {stdout}");
}

#[test]
fn policy_slot_records_referenced_ident_as_a_string() {
    build_axon();
    let dir = temp_dir("policy");
    let prog = r#"
agent T() {
    policy: support_policy,

    on which() -> String uses { Audit } {
        self.policy
    }
}

fn main() uses { Spawn, Audit, Console } {
    let t = spawn T()
    print(t.which())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim_end().ends_with("support_policy"),
        "stdout: {stdout}"
    );
}

#[test]
fn strategy_slot_carries_string_literal_through_to_handler() {
    build_axon();
    let dir = temp_dir("strategy");
    let prog = r#"
agent T() {
    strategy: "ReAct",

    on which() -> String { self.strategy }
}

fn main() uses { Spawn, Console } {
    let t = spawn T()
    print(t.which())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_end().ends_with("ReAct"), "stdout: {stdout}");
}

#[test]
fn all_four_slots_compose_in_one_agent_block() {
    build_axon();
    let dir = temp_dir("compose");
    let prog = r#"
tool s(x: String) -> String uses { Net } { x }

agent T() {
    uses_tools: [s],
    memory: local_memory(),
    policy: my_policy,
    strategy: "PlanExec",

    on info() -> String uses { Tool, Memory } {
        ("ok n=" + str(list_len(self.tools))
            + " p=" + self.policy
            + " s=" + self.strategy)
    }
}

fn main() uses { Spawn, Tool, Memory, Console } {
    let t = spawn T()
    print(t.info())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n=1"), "{stdout}");
    assert!(stdout.contains("p=my_policy"), "{stdout}");
    assert!(stdout.contains("s=PlanExec"), "{stdout}");
}

#[test]
fn user_state_field_wins_when_only_state_declared() {
    // Sanity check on the no-conflict case: a `state tools` field
    // alone produces a list with whatever the state initializer says.
    build_axon();
    let dir = temp_dir("user_state_only");
    let prog = r#"
agent T() {
    state tools: List<dyn> = []

    on count() -> Int { list_len(self.tools) }
}

fn main() uses { Spawn, Console } {
    let t = spawn T()
    print_int(t.count())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_end().ends_with("0"), "stdout: {stdout}");
}

#[test]
fn user_state_field_wins_over_same_named_slot_default() {
    // §35.6 verification fix C1: the conflict the prior fixture failed
    // to exercise. Both `uses_tools: [a, b]` AND `state tools = []` are
    // declared. The user's explicit state must win — count = 0, not 2.
    // This pins the precedence rule documented in eval.rs eval_spawn
    // and in tyck state_field_sigs.
    build_axon();
    let dir = temp_dir("user_wins_conflict");
    let prog = r#"
tool a(s: String) -> String uses { Net } { s }
tool b(s: String) -> String uses { Net } { s }

agent T() {
    uses_tools: [a, b],
    state tools: List<dyn> = []

    on count() -> Int { list_len(self.tools) }
}

fn main() uses { Spawn, Console } {
    let t = spawn T()
    print_int(t.count())
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim_end().ends_with("0"),
        "user state field must win over uses_tools slot, got: {stdout}"
    );
}
