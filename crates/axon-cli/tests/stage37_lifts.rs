//! Stage 37 — acceptance tests for the four lifts: contextual keywords,
//! `parallel { }` arm restriction, `select` real timeout, `for await`
//! real stream consumption.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

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
    p.push(format!("axon-stage37-{name}-{}-{ts}", std::process::id()));
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

// =========================================================================
// §37.A — Contextual keywords
// =========================================================================

#[test]
fn contextual_keywords_usable_as_let_bindings() {
    build_axon();
    let dir = temp_dir("ctx_let");
    let prog = r#"
fn main() uses { Console } {
    let prompt = "hello"
    let model = "claude"
    let memory = "session-7"
    let tool = "calculator"
    let agent = "assistant"
    print(prompt)
    print(model)
    print(memory)
    print(tool)
    print(agent)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(
        lines,
        ["hello", "claude", "session-7", "calculator", "assistant"],
        "got: {lines:?}"
    );
}

#[test]
fn contextual_keyword_in_ask_target_position() {
    build_axon();
    let dir = temp_dir("ctx_ask");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let model = mock_model("fixed", "ok")
    let r = ask model { user: "hi" }
    print(r)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ok"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn item_position_dispatch_still_works_for_soft_keywords() {
    build_axon();
    let dir = temp_dir("ctx_item");
    // `tool` at item-position parses as a tool decl; `tool` at let-position
    // parses as a binding. The lexer emits Keyword(Tool) in both cases;
    // the parser disambiguates by context.
    let prog = r#"
tool calc(q: String) -> String uses { Net } { "= 42" }
fn main() uses { Console } {
    let tool = "the variable, not the item"
    print(tool)
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("variable"),
        "expected the binding value to print; got: {stdout}"
    );
}

#[test]
fn reserved_keywords_still_cannot_be_used_as_idents() {
    // Stage 37 is opinionated about WHICH keywords become soft — only
    // `prompt/model/memory/tool/agent`. Control-flow + type-system words
    // (`if`, `while`, `fn`, `let`, etc.) stay reserved.
    build_axon();
    let dir = temp_dir("ctx_reserved");
    let prog = r#"
fn main() uses { Console } {
    let if = 1
    print_int(if)
}
"#;
    let out = run_src(&dir, prog);
    assert!(
        !out.status.success(),
        "`let if = ...` must still be a parse error: {:?}",
        out
    );
}

// =========================================================================
// §37.B — parallel { } arm lift
// =========================================================================

#[test]
fn parallel_general_path_runs_non_ask_arms_sequentially() {
    build_axon();
    let dir = temp_dir("p_general");
    let prog = r#"
fn main() uses { Console } {
    let xs = parallel {
        1 + 2,
        2 * 5,
        100 - 7,
    }
    print_int(list_get(xs, 0))
    print_int(list_get(xs, 1))
    print_int(list_get(xs, 2))
}
"#;
    let out = run_src(&dir, prog);
    assert!(
        out.status.success(),
        "Stage 37 lift should run non-ask arms sequentially: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["3", "10", "93"]);
}

#[test]
fn parallel_arms_can_be_user_functions_calling_ask() {
    build_axon();
    let dir = temp_dir("p_fn");
    let prog = r#"
fn research(q: String) -> String uses { LLM, Net } {
    let m = mock_model("fixed", "research: " + q)
    ask m { user: q }
}
fn summarize(text: String) -> String uses { LLM, Net } {
    let m = mock_model("fixed", "summary: " + text)
    ask m { user: text }
}
fn main() uses { Console, LLM, Net } {
    let xs = parallel {
        research("ferrets"),
        summarize("research notes"),
    }
    print(list_get(xs, 0))
    print(list_get(xs, 1))
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(
        lines,
        [
            "research: ferrets",
            "summary: research notes",
        ]
    );
}

#[test]
fn parallel_all_ask_fast_path_still_overlaps_in_wall_time() {
    // Stage 37 must NOT regress Stage 36's fast-path parallelism. Two
    // bare-ask arms at 200ms each must still finish in < 700ms.
    build_axon();
    let dir = temp_dir("p_fast");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m1 = mock_model_slow("a", 200)
    let m2 = mock_model_slow("b", 200)
    let xs = parallel {
        ask m1 { user: "q1" },
        ask m2 { user: "q2" },
    }
    print(list_get(xs, 0))
    print(list_get(xs, 1))
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    assert!(
        elapsed < Duration::from_millis(700),
        "Stage 37 must preserve Stage 36's fast path; wall {elapsed:?}"
    );
}

#[test]
fn parallel_general_path_first_arm_error_short_circuits() {
    // Sequential semantics: the first arm to error halts the whole
    // parallel block. No subsequent arms run, no list returned.
    build_axon();
    let dir = temp_dir("p_err");
    let prog = r#"
fn main() uses { Console } {
    let _ = parallel {
        1 + 2,
        panic("forced"),
        3 * 4,
    }
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("forced"),
        "stderr should surface the underlying panic; got: {stderr}"
    );
}

// =========================================================================
// §37.C — select async timeout (real wait)
// =========================================================================

#[test]
fn select_timeout_actually_waits_the_declared_duration() {
    // Stage 36: timeout fired immediately. Stage 37: timeout waits.
    // 300ms timeout, no producer → should take ~300ms wall-clock.
    build_axon();
    let dir = temp_dir("sel_wait");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    select {
        v = recv(c) => { print(v) }
        _ = timeout(300) => { print("timed out") }
    }
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("timed out"), "stdout: {stdout}");
    // Allow some startup slack but require at least the declared wait.
    assert!(
        elapsed >= Duration::from_millis(250),
        "Stage 37 timeout must actually wait — got {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1500),
        "wait should not balloon past the declared duration + startup: {elapsed:?}"
    );
}

#[test]
fn select_takes_minimum_of_multiple_timeouts() {
    // Two timeout arms; the shortest deadline fires first.
    build_axon();
    let dir = temp_dir("sel_min");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    select {
        v = recv(c) => { print(v) }
        _ = timeout(500) => { print("five hundred") }
        _ = timeout(100) => { print("one hundred") }
    }
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("one hundred"),
        "shortest deadline must fire first; got: {stdout}"
    );
    assert!(
        elapsed < Duration::from_millis(1200),
        "must not wait the longer deadline: {elapsed:?}"
    );
}

#[test]
fn select_ready_recv_wins_without_waiting() {
    // A channel that already has a value present should win the select
    // without any timeout-related wait. This pins that the post-recv
    // logic still short-circuits before the new sleep.
    build_axon();
    let dir = temp_dir("sel_ready");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.send("hello")
    select {
        v = recv(c) => { print(v) }
        _ = timeout(5000) => { print("would-be-bad") }
    }
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(
        elapsed < Duration::from_millis(1500),
        "ready recv must short-circuit, not wait the timeout: {elapsed:?}"
    );
}

#[test]
fn select_channel_expression_evaluated_exactly_once_even_when_timeout_fires() {
    // §37.6 STAGE37-001 regression — the channel expression in a `recv(...)`
    // arm must run exactly once per select execution. Before the fix, the
    // pre-sleep and post-sleep probes both re-evaluated the channel
    // expression, causing effectful constructors to run twice.
    build_axon();
    let dir = temp_dir("sel_once");
    let prog = r#"
fn make_chan() uses { Console, Channel } {
    print("make_chan called")
    chan()
}
fn main() uses { Console, Channel } {
    select {
        v = recv(make_chan()) => { print("got value") }
        _ = timeout(80) => { print("timeout fired") }
    }
}
"#;
    let out = run_src(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let call_count = stdout.matches("make_chan called").count();
    assert_eq!(
        call_count, 1,
        "STAGE37-001 regressed: channel expression evaluated {call_count}x. stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("timeout fired"),
        "timeout must still fire: {stdout}"
    );
}

#[test]
fn select_timeout_above_one_hour_is_rejected() {
    build_axon();
    let dir = temp_dir("sel_cap");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    select {
        v = recv(c) => { print(v) }
        _ = timeout(3600001) => { print("nope") }
    }
}
"#;
    let out = run_src(&dir, prog);
    assert!(!out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("3600000") || stderr.contains("1-hour"),
        "expected bound message; got: {stderr}"
    );
}

// =========================================================================
// §37.D — for await stream wait
// =========================================================================

#[test]
fn for_await_drains_chan_and_exits_within_window() {
    // Stage 37 used a 50ms post-drain poll heuristic. Stage 38 replaced
    // that with closed-flag-aware semantics: open channels wait for new
    // values up to a 5-minute safety budget, closed-and-empty exits
    // immediately. The fixture must close the channel so the for-await
    // exits cleanly under Stage 38's new semantic.
    build_axon();
    let dir = temp_dir("forawait_drain");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    c.send("alpha")
    c.send("beta")
    c.send("gamma")
    c.close()
    for await v in c {
        print(v)
    }
    print("done")
}
"#;
    let start = Instant::now();
    let out = run_src(&dir, prog);
    let elapsed = start.elapsed();
    assert!(out.status.success(), "{:?}", out);
    let lines: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .collect();
    assert_eq!(lines, ["alpha", "beta", "gamma", "done"]);
    // The §37 post-drain wait should be quick — empty drain budget is 50ms.
    // The whole program should finish well under 1.5s including startup.
    assert!(
        elapsed < Duration::from_millis(1500),
        "for-await should drain + exit promptly: {elapsed:?}"
    );
}
