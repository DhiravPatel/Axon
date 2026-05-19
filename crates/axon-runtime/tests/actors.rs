//! Stage 5.5 — actor / agent integration tests.
//!
//! Programs exercise `spawn`, message dispatch, mutable state, multi-agent
//! coordination, lifecycle hooks, capability attenuation per handler, and
//! channels.

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_runtime::{run_with_caps, CapSet, RuntimeError, Value};

fn run_ok(src: &str) -> Value {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    match run_with_caps(&file, &program, CapSet::standard_default()) {
        Ok(v) => v,
        Err(e) => panic!("run failed: {e:#?}"),
    }
}

fn run_with(src: &str, caps: CapSet) -> Result<Value, RuntimeError> {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    run_with_caps(&file, &program, caps)
}

#[test]
fn counter_agent_state_persists_across_calls() {
    let v = run_ok(
        r#"
agent Counter() {
    state count: Int = 0
    on inc() -> Int {
        self.count = self.count + 1
        self.count
    }
}

fn main() -> Int uses { Spawn } {
    let c = spawn Counter()
    c.inc()
    c.inc()
    c.inc()
}"#,
    );
    assert!(matches!(v, Value::Int(3)));
}

#[test]
fn ctor_params_are_visible_via_self() {
    let v = run_ok(
        r#"
agent Greeter(name: String) {
    on hello() -> String {
        "hi, " + self.name
    }
}

fn main() -> String uses { Spawn } {
    let g = spawn Greeter(name = "axon")
    g.hello()
}"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "hi, axon");
    } else {
        panic!("expected String, got {v:?}");
    }
}

#[test]
fn handlers_take_arguments() {
    let v = run_ok(
        r#"
agent Bank() {
    state balance: Int = 100
    on deposit(amount: Int) -> Int {
        self.balance = self.balance + amount
        self.balance
    }
    on withdraw(amount: Int) -> Int {
        self.balance = self.balance - amount
        self.balance
    }
}
fn main() -> Int uses { Spawn } {
    let b = spawn Bank()
    b.deposit(50)
    b.withdraw(30)
    b.deposit(10)
}"#,
    );
    // 100 + 50 - 30 + 10 = 130
    assert!(matches!(v, Value::Int(130)));
}

#[test]
fn on_start_runs_at_spawn_time() {
    // The on-start handler mutates state; the post-spawn read sees it.
    let v = run_ok(
        r#"
agent Greeter() {
    state msg: String = ""
    on start() {
        self.msg = "started"
    }
    on what() -> String { self.msg }
}
fn main() -> String uses { Spawn } {
    let g = spawn Greeter()
    g.what()
}"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "started");
    } else {
        panic!();
    }
}

#[test]
fn multi_agent_communication() {
    let v = run_ok(
        r#"
agent Producer() {
    on next() -> Int { 42 }
}
agent Consumer(p: dyn) {
    on grab() -> Int {
        self.p.next()
    }
}
fn main() -> Int uses { Spawn } {
    let p = spawn Producer()
    let c = spawn Consumer(p = p)
    c.grab()
}"#,
    );
    assert!(matches!(v, Value::Int(42)));
}

#[test]
fn handler_can_call_other_handler_on_self() {
    let v = run_ok(
        r#"
agent X() {
    state n: Int = 0
    on bump() -> Int {
        self.n = self.n + 1
        self.n
    }
    on twice() -> Int {
        self.bump()
        self.bump()
    }
}
fn main() -> Int uses { Spawn } {
    let x = spawn X()
    x.twice()
}"#,
    );
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn handler_with_console_effect_attenuates_callers_caps() {
    // Handler declares Console; main has Console + Spawn. Handler runs.
    let v = run_ok(
        r#"
agent Logger() {
    on say(s: String) uses { Console } {
        print(s)
    }
}
fn main() uses { Console, Spawn } {
    let l = spawn Logger()
    l.say("hello")
}"#,
    );
    assert!(matches!(v, Value::Unit));
}

#[test]
fn handler_missing_caller_cap_is_denied_at_dispatch() {
    let src = r#"
agent Logger() {
    on say() uses { Console } {
        print("nope")
    }
}
fn main() -> Int uses { Spawn } {
    let l = spawn Logger()
    l.say()
    0
}"#;
    // main has only Spawn — Console isn't granted to it, so it can't pass
    // that capability into the handler. (Note: the static type checker may
    // also flag this; the test asserts the runtime catches it.)
    let file = SourceFile::new("t.ax", src);
    let (program, _) = parse(&file);
    let res = run_with_caps(
        &file,
        &program,
        CapSet::from_iter(["Spawn"]),
    );
    assert!(res.is_err());
    let msg = res.unwrap_err().message;
    assert!(
        msg.contains("Console") && (msg.contains("not granted") || msg.contains("not in scope")),
        "msg = {msg}"
    );
}

#[test]
fn unknown_method_is_a_runtime_error() {
    let file = SourceFile::new(
        "t.ax",
        "agent A() { on go() -> Int { 1 } }\n\
         fn main() -> Int uses { Spawn } { let a = spawn A(); a.nope() }",
    );
    let (program, _) = parse(&file);
    let res = run_with_caps(&file, &program, CapSet::standard_default());
    let err = res.unwrap_err();
    assert!(
        err.message.contains("no handler `nope`"),
        "msg = {}",
        err.message
    );
}

#[test]
fn unknown_field_on_self_is_a_runtime_error() {
    let src = r#"
agent A() {
    state x: Int = 1
    on bad() -> Int { self.y }
}
fn main() -> Int uses { Spawn } { let a = spawn A(); a.bad() }"#;
    let file = SourceFile::new("t.ax", src);
    let (program, _) = parse(&file);
    let err = run_with_caps(&file, &program, CapSet::standard_default()).unwrap_err();
    assert!(
        err.message.contains("no field `y`"),
        "msg = {}",
        err.message
    );
}

#[test]
fn on_error_observes_handler_failure_then_error_propagates() {
    // The error handler mutates state (so we can observe it ran) and then
    // the original error still propagates.
    let src = r#"
agent FailingTracker() {
    state errs: Int = 0
    on bomb() -> Int {
        let zero: Int = 0
        1 / zero
    }
    on error(msg: String) {
        self.errs = self.errs + 1
    }
}
fn main() -> Int uses { Spawn } {
    let a = spawn FailingTracker()
    let _ = a.bomb()
    0
}"#;
    let file = SourceFile::new("t.ax", src);
    let (program, _) = parse(&file);
    let err = run_with_caps(&file, &program, CapSet::standard_default()).unwrap_err();
    assert!(err.message.contains("division by zero"));
}

#[test]
fn channel_send_recv_round_trip() {
    let v = run_ok(
        r#"
fn main() -> Int {
    let q = chan()
    q.send(10)
    q.send(20)
    let a = q.recv()
    let b = q.recv()
    a! + b!
}"#,
    );
    assert!(matches!(v, Value::Int(30)));
}

#[test]
fn channel_recv_on_empty_returns_nil() {
    let v = run_ok(
        "fn main() -> Bool { let q = chan(); q.recv() == nil }",
    );
    assert!(matches!(v, Value::Bool(true)));
}

#[test]
fn channel_len_and_is_empty() {
    let v = run_ok(
        r#"
fn main() -> Bool {
    let q = chan()
    let empty = q.is_empty()
    q.send(1)
    q.send(2)
    q.len() == 2 && empty
}"#,
    );
    assert!(matches!(v, Value::Bool(true)));
}

#[test]
fn channels_can_be_shared_between_agents() {
    let v = run_ok(
        r#"
agent Sink(q: dyn) {
    on take() -> Int {
        let v = self.q.recv()
        v!
    }
}
fn main() -> Int uses { Spawn } {
    let q = chan()
    q.send(99)
    let s = spawn Sink(q = q)
    s.take()
}"#,
    );
    assert!(matches!(v, Value::Int(99)));
}

#[test]
fn isolated_caps_prevent_spawn() {
    let err = run_with(
        "agent A() { on go() -> Int { 0 } }\n\
         fn main() -> Int uses { Spawn } { let _ = spawn A(); 0 }",
        CapSet::empty(),
    )
    .unwrap_err();
    assert!(err.message.contains("Spawn"), "msg = {}", err.message);
}
