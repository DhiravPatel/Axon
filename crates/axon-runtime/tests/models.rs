//! Stage 6 — Model / Memory / ask / generate / plan integration tests.
//!
//! Programs use the `mock_model(...)` built-in so tests don't require an
//! API key or network access. The mock provider returns deterministic
//! outputs; tests assert on those plus on the capability-gating behavior.

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
fn ask_with_mock_echo_returns_user_text() {
    let v = run_ok(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model()
    ask m { user: "hello world" }
}"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "hello world");
    } else {
        panic!("expected String, got {v:?}");
    }
}

#[test]
fn ask_with_fixed_returns_canned_text() {
    let v = run_ok(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model("fixed", "the secret is 42")
    ask m { user: "ignored" }
}"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "the secret is 42");
    } else {
        panic!();
    }
}

#[test]
fn plan_supports_system_and_user_slots() {
    let v = run_ok(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model()
    plan with m {
        system: "You are a helper."
        user:   "Question: what year is it?"
    }
}"#,
    );
    // The echo provider concatenates user messages — the user slot wins.
    if let Value::String(s) = v {
        assert!(s.contains("Question"));
    } else {
        panic!();
    }
}

#[test]
fn missing_llm_cap_denies_ask() {
    let err = run_with(
        r#"
fn main() -> String uses { Net } {
    let m = mock_model()
    ask m { user: "hi" }
}"#,
        CapSet::from_iter(["Net"]),
    )
    .unwrap_err();
    assert!(err.message.contains("LLM"), "msg = {}", err.message);
}

#[test]
fn missing_net_cap_denies_ask() {
    let err = run_with(
        r#"
fn main() -> String uses { LLM } {
    let m = mock_model()
    ask m { user: "hi" }
}"#,
        CapSet::from_iter(["LLM"]),
    )
    .unwrap_err();
    assert!(err.message.contains("Net"), "msg = {}", err.message);
}

#[test]
fn generate_with_simple_schema_returns_record() {
    let v = run_ok(
        r#"
schema Answer { text: String, citations: [String] }

fn main() -> Answer uses { LLM, Net } {
    let m = mock_model()
    generate<Answer>(m, "what's the capital of France?")
}"#,
    );
    if let Value::Record(r) = v {
        let r = r.borrow();
        assert!(r.iter().any(|(k, _)| k == "text"));
        assert!(r.iter().any(|(k, _)| k == "citations"));
    } else {
        panic!("expected Record, got {v:?}");
    }
}

#[test]
fn generate_with_primitive_schema_returns_primitive() {
    let v = run_ok(
        r#"
fn main() -> Int uses { LLM, Net } {
    let m = mock_model()
    generate<Int>(m, "What's 2+2?")
}"#,
    );
    assert!(matches!(v, Value::Int(0)));
}

#[test]
fn local_memory_recall_after_store() {
    let v = run_ok(
        r#"
fn main() -> Bool {
    let kb = local_memory()
    kb.store("Alpha is the first letter")
    kb.store("Beta is the second letter")
    kb.store("Gamma is the third")
    let hits = kb.recall("Beta", 5)
    hits.len() == 1
}"#,
    );
    assert!(matches!(v, Value::Bool(true)));
}

#[test]
fn local_memory_recall_respects_k() {
    let v = run_ok(
        r#"
fn main() -> Int {
    let kb = local_memory()
    kb.store("aa")
    kb.store("ab")
    kb.store("ac")
    let hits = kb.recall("a", 2)
    hits.len()
}"#,
    );
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn memory_slot_in_ask_is_passed_through() {
    let v = run_ok(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model()
    let kb = local_memory()
    kb.store("the password is 1234")
    let hits = kb.recall("password", 5)
    ask m {
        system: "Answer with the password from memory."
        memory: hits
        user:   "What is the password?"
    }
}"#,
    );
    if let Value::String(s) = v {
        assert!(s.contains("What is the password"));
    } else {
        panic!();
    }
}

#[test]
fn model_declaration_binds_a_global() {
    let v = run_ok(
        r#"
model my_model = mock_model("fixed", "from-decl")

fn main() -> String uses { LLM, Net } {
    ask my_model { user: "anything" }
}"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "from-decl");
    } else {
        panic!();
    }
}

#[test]
fn memory_declaration_binds_a_global() {
    let v = run_ok(
        r#"
memory kb = local_memory()

fn main() -> Int {
    kb.store("hi")
    kb.store("there")
    kb.len()
}"#,
    );
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn calling_a_model_via_method_is_a_helpful_error() {
    let file = SourceFile::new(
        "t.ax",
        r#"
fn main() -> dyn uses { LLM, Net } {
    let m = mock_model()
    m.complete("hi")
}"#,
    );
    let (program, _) = parse(&file);
    let err = run_with_caps(&file, &program, CapSet::standard_default()).unwrap_err();
    assert!(
        err.message.contains("ask") && err.message.contains("generate"),
        "msg = {}",
        err.message
    );
}

#[test]
fn ask_against_a_non_model_is_a_runtime_error() {
    let err = run_with(
        r#"
fn main() -> String uses { LLM, Net } {
    let not_a_model: dyn = 42
    ask not_a_model { user: "x" }
}"#,
        CapSet::standard_default(),
    )
    .unwrap_err();
    assert!(err.message.contains("Model"), "msg = {}", err.message);
}

#[test]
fn researcher_pattern_runs_end_to_end_against_mock() {
    // Lifted from the README's marquee example but using the mock model
    // and a `local_memory`. Demonstrates: spawn + state + ctor params +
    // ask + memory.recall slotted into the prompt + capability declarations.
    let src = r#"
agent Researcher(m: dyn, kb: dyn) {
    on inquiry(question: String) -> String uses { LLM, Net, Memory } {
        let ctx = self.kb.recall(question, 6)
        ask self.m {
            system: "Cite every claim."
            memory: ctx
            user:   question
        }
    }
}

fn main() -> String uses { Spawn, LLM, Net, Memory } {
    let m = mock_model()
    let kb = local_memory()
    kb.store("the EU AI Act introduced obligations in 2024")
    let r = spawn Researcher(m = m, kb = kb)
    r.inquiry("What changed in the EU AI Act in 2025?")
}"#;
    let v = run_ok(src);
    if let Value::String(s) = v {
        assert!(s.contains("EU AI Act"));
    } else {
        panic!();
    }
}
