//! Stage 19 — `for await`, `select`, and `plan` slot enhancements
//! exercised through the binary.

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
    p.push(format!("axon-stage19-{name}-{pid}-{ts}"));
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

// ---------- for await ----------------------------------------------------

#[test]
fn for_await_iterates_a_list_like_a_stream() {
    build_axon();
    let dir = temp_dir("for_await_list");
    let prog = r#"
fn main() uses { Console } {
    let xs = list_new(10, 20, 30)
    for await x in xs {
        print_int(x)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["10", "20", "30"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn for_await_drains_a_chan_until_empty() {
    // Stage 38 changed the for-await semantic: an OPEN channel waits for
    // new values up to a 5-minute safety budget. The fixture must close
    // the channel so the loop exits cleanly. Pre-Stage-38 tests relied on
    // the §37.D 50ms post-drain poll, which is gone.
    build_axon();
    let dir = temp_dir("for_await_chan");
    let prog = r#"
fn main() uses { Console } {
    let c = chan()
    // .send is the chan-built-in method; push three values then drain.
    c.send(1)
    c.send(2)
    c.send(3)
    c.close()
    for await v in c {
        print_int(v)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn for_await_break_and_continue_work() {
    build_axon();
    let dir = temp_dir("for_await_ctrl");
    let prog = r#"
fn main() uses { Console } {
    let xs = list_new(1, 2, 3, 4, 5)
    for await x in xs {
        if x == 2 { continue }
        if x == 4 { break }
        print_int(x)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["1", "3"]);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- select -------------------------------------------------------

#[test]
fn select_picks_first_ready_channel_in_order() {
    build_axon();
    let dir = temp_dir("select_ready");
    let prog = r#"
fn main() uses { Console } {
    let a = chan()
    let b = chan()
    b.send("hello-from-b")
    select {
        msg = recv(a) => print(msg)
        msg = recv(b) => print(msg)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "hello-from-b");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn select_falls_to_else_when_no_channel_is_ready() {
    build_axon();
    let dir = temp_dir("select_else");
    let prog = r#"
fn main() uses { Console } {
    let empty = chan()
    select {
        msg = recv(empty) => print(msg)
        else => print("nothing ready")
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "nothing ready");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn select_falls_to_timeout_when_no_channel_ready_and_no_else() {
    build_axon();
    let dir = temp_dir("select_timeout");
    let prog = r#"
fn main() uses { Console } {
    let empty = chan()
    select {
        msg = recv(empty) => print(msg)
        _ = timeout(5) => print("timed out")
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "timed out");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn select_arm_order_determines_winner_when_multiple_ready() {
    build_axon();
    let dir = temp_dir("select_order");
    let prog = r#"
fn main() uses { Console } {
    let a = chan()
    let b = chan()
    a.send("from-a")
    b.send("from-b")
    select {
        m = recv(a) => print(m)
        m = recv(b) => print(m)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    // Declaration-order tiebreak: `a` wins.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "from-a");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn select_with_no_match_errors_cleanly() {
    build_axon();
    let dir = temp_dir("select_no_match");
    let prog = r#"
fn main() {
    let empty = chan()
    select {
        msg = recv(empty) => print(msg)
    }
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have errored");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no channel was ready"),
        "expected select error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- plan: max_steps + output ------------------------------------

#[test]
fn plan_max_steps_invalid_value_rejected() {
    build_axon();
    let dir = temp_dir("plan_max_steps_bad");
    // `mock_model` returns canned output; `plan` with `max_steps = 0` is
    // an invalid request — verify the runtime catches it before any LLM
    // call happens.
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "static answer")
    let _ = plan with m {
        system: "test"
        user: "hi"
        max_steps: 0
    } await
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have rejected max_steps = 0");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("max_steps"),
        "expected max_steps error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_output_slot_parses_json_result_as_record() {
    build_axon();
    let dir = temp_dir("plan_output_json");
    // mock_model echoes whatever string it was constructed with. We give
    // it a JSON object so the `output:` post-parse can hand back a Record.
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", `{"answer":"yes","score":3}`)
    let r = plan with m {
        system: "respond with JSON"
        user: "go"
        output: "Answer"
    } await
    print(r.answer)
    print_int(r.score)
}
"#;
    let out = run_in(&dir, prog);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "yes");
    assert_eq!(lines[1], "3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_output_slot_with_invalid_json_surfaces_error() {
    build_axon();
    let dir = temp_dir("plan_output_bad");
    let prog = r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "not valid json at all")
    let _ = plan with m {
        system: "respond with JSON"
        user: "go"
        output: "Answer"
    } await
}
"#;
    let out = run_in(&dir, prog);
    assert!(!out.status.success(), "should have errored on bad JSON");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("valid JSON") || stderr.contains("parse"),
        "expected JSON parse error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
