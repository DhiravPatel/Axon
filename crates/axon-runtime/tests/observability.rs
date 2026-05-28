//! Stage 7 — tracing, budgets, record/replay.
//!
//! Tracing tests inspect the in-memory tracer after a run; budget tests
//! exercise the `with budget(...)` scope and confirm the runtime halts
//! before the next call once a ceiling is breached; record/replay tests
//! prove a run can be captured and replayed byte-for-byte without
//! contacting the original provider.

use axon_diag::SourceFile;
use axon_models::{ChatRequest, ChatResponse, ContentBlock, ModelProvider, ProviderError, StopReason, TokenUsage};
use axon_parser::parse;
use axon_runtime::{
    Budget, CapSet, Interpreter, RecordedEvent, Recording, Replay, SpanKind, Value,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

fn load(src: &str) -> (axon_diag::SourceFile, axon_ast::Program) {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    (file, program)
}

// ===========================================================================
// Tracing
// ===========================================================================

#[test]
fn tracer_records_ask_span_with_model_attribute() {
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model("fixed", "hi")
    ask m { user: "hello" }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    interp.enable_tracing();
    interp.load_program(&program);
    interp.run_main().expect("run");

    let tracer = interp.take_tracer().expect("tracer enabled");
    let spans = tracer.spans();
    let ask = spans
        .iter()
        .find(|s| s.kind == SpanKind::Ask)
        .expect("ask span");
    assert!(ask.attributes.contains_key("model"));
    assert!(ask.duration_ms().is_some());
}

#[test]
fn tracer_records_with_span_user_scope() {
    let (_file, program) = load(
        r#"
fn main() -> Int {
    with span("compute", n = 3) {
        1 + 2
    }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    interp.enable_tracing();
    interp.load_program(&program);
    let v = interp.run_main().expect("run");
    assert!(matches!(v, Value::Int(3)));

    let tracer = interp.take_tracer().unwrap();
    let scope = tracer
        .spans()
        .iter()
        .find(|s| s.kind == SpanKind::UserScope)
        .expect("with-span recorded");
    assert_eq!(scope.name, "compute");
    assert!(scope.attributes.contains_key("n"));
}

#[test]
fn tracer_jsonl_is_one_object_per_line() {
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    let m = mock_model()
    ask m { user: "x" }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    interp.enable_tracing();
    interp.load_program(&program);
    interp.run_main().unwrap();
    let lines = interp.take_tracer().unwrap().to_jsonl();
    let lc = lines.lines().count();
    assert!(lc >= 1, "expected at least one trace line, got {lc}");
    for line in lines.lines() {
        // Cheap shape check: each line is a JSON object.
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
    }
}

// ===========================================================================
// Budgets
// ===========================================================================

/// A model provider that returns a fixed token usage per call so budget
/// arithmetic is deterministic.
struct FixedUsageProvider {
    usd_per_call: f64,
    tokens_per_call: u32,
    response: String,
}

impl ModelProvider for FixedUsageProvider {
    fn name(&self) -> &str {
        "fixed-usage"
    }
    fn complete(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            content: self.response.clone(),
            blocks: vec![ContentBlock::Text(self.response.clone())],
            structured: None,
            tool_calls: Vec::new(),
            usage: TokenUsage {
                input_tokens: 0,
                output_tokens: self.tokens_per_call,
                cost_usd: self.usd_per_call,
            },
            stop_reason: StopReason::EndTurn,
        })
    }
}

fn install_fixed_provider(
    interp: &mut Interpreter,
    name: &str,
    usd: f64,
    tokens: u32,
    response: &str,
) {
    let provider = Arc::new(FixedUsageProvider {
        usd_per_call: usd,
        tokens_per_call: tokens,
        response: response.to_owned(),
    });
    interp.globals.define(name, Value::Model(provider));
}

#[test]
fn budget_under_ceiling_allows_calls() {
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    with budget(tokens = 1000) {
        ask my_model { user: "first" }
    }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    install_fixed_provider(&mut interp, "my_model", 0.0, 50, "ok");
    interp.load_program(&program);
    let v = interp.run_main().unwrap();
    if let Value::String(s) = v {
        assert_eq!(&*s, "ok");
    } else {
        panic!();
    }
}

#[test]
fn budget_token_ceiling_denies_next_call() {
    // First call spends 1000 tokens (right at the ceiling). The second
    // call's precheck should fire before any HTTP work.
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    with budget(tokens = 100) {
        let a = ask my_model { user: "first" }
        let b = ask my_model { user: "second" }
        b
    }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    install_fixed_provider(&mut interp, "my_model", 0.0, 150, "ok");
    interp.load_program(&program);
    let err = interp.run_main().unwrap_err();
    assert!(
        err.message.contains("token budget exceeded"),
        "msg = {}",
        err.message
    );
}

#[test]
fn budget_usd_ceiling_denies_next_call() {
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    with budget(usd = 0.01) {
        let a = ask my_model { user: "first" }
        let b = ask my_model { user: "second" }
        b
    }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    install_fixed_provider(&mut interp, "my_model", 0.02, 0, "ok");
    interp.load_program(&program);
    let err = interp.run_main().unwrap_err();
    assert!(
        err.message.contains("USD budget exceeded"),
        "msg = {}",
        err.message
    );
}

#[test]
fn nested_budgets_inner_exceeds_first() {
    // Inner budget is the tightest; it should fail first.
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    with budget(tokens = 1000) {
        with budget(tokens = 50) {
            let a = ask my_model { user: "first" }
            let b = ask my_model { user: "second" }
            b
        }
    }
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    install_fixed_provider(&mut interp, "my_model", 0.0, 100, "ok");
    interp.load_program(&program);
    let err = interp.run_main().unwrap_err();
    assert!(err.message.contains("token budget"));
}

#[test]
fn budget_on_exceeded_lambda_recovers() {
    // The `on_exceeded` clause turns the budget breach into a value the
    // caller can recover with — useful for "log and degrade gracefully".
    let (_file, program) = load(
        r#"
fn main() -> String uses { LLM, Net } {
    with budget(tokens = 50) {
        let a = ask my_model { user: "first" }
        let b = ask my_model { user: "second" }
        b
    } on_exceeded |msg| "budget hit"
}"#,
    );
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    install_fixed_provider(&mut interp, "my_model", 0.0, 100, "ok");
    interp.load_program(&program);
    let v = interp.run_main().expect("on_exceeded should recover");
    if let Value::String(s) = v {
        assert_eq!(&*s, "budget hit");
    } else {
        panic!();
    }
}

// ===========================================================================
// Record / Replay
// ===========================================================================

/// A provider that fails after N successful calls — used to prove replay
/// doesn't actually hit it. Uses `AtomicU32` so it satisfies the
/// `Send + Sync` bound on `ModelProvider`.
struct CountingProvider {
    counter: AtomicU32,
    limit: u32,
}

impl ModelProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }
    fn complete(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let n = self.counter.fetch_add(1, AtomicOrdering::SeqCst);
        if n >= self.limit {
            return Err(ProviderError::Network(
                "should not have been called (replay leaked)".into(),
            ));
        }
        let body = format!("response #{n}");
        Ok(ChatResponse {
            content: body.clone(),
            blocks: vec![ContentBlock::Text(body)],
            structured: None,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            stop_reason: StopReason::EndTurn,
        })
    }
}

#[test]
fn recording_then_replay_roundtrips() {
    let src = r#"
fn main() -> String uses { LLM, Net } {
    let a = ask my_model { user: "q1" }
    let b = ask my_model { user: "q2" }
    a + " | " + b
}"#;
    let (_file, program) = load(src);

    // Record run.
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    let provider = Arc::new(CountingProvider {
        counter: AtomicU32::new(0),
        limit: 100,
    });
    interp.globals.define("my_model", Value::Model(provider));
    interp.enable_recording();
    interp.load_program(&program);
    let recorded = interp.run_main().unwrap();
    let rec = interp.take_recording().expect("recording");
    assert_eq!(rec.events.len(), 2);

    // Replay run — provider that *panics* if called.
    let mut interp2 = Interpreter::with_caps(CapSet::standard_default());
    let leaky = Arc::new(CountingProvider {
        counter: AtomicU32::new(0),
        limit: 0, // any call errors out
    });
    interp2.globals.define("my_model", Value::Model(leaky));
    interp2.enable_replay(rec);
    interp2.load_program(&program);
    let replayed = interp2.run_main().unwrap();

    assert_eq!(format!("{recorded}"), format!("{replayed}"));
}

#[test]
fn recording_serializes_and_deserializes() {
    let src = r#"
fn main() -> String uses { LLM, Net } {
    ask my_model { user: "x" }
}"#;
    let (_file, program) = load(src);
    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    interp.globals.define(
        "my_model",
        Value::Model(Arc::new(CountingProvider {
            counter: AtomicU32::new(0),
            limit: 100,
        })),
    );
    interp.enable_recording();
    interp.load_program(&program);
    interp.run_main().unwrap();
    let rec = interp.take_recording().unwrap();

    let json = rec.to_json();
    let serialized = serde_json::to_string(&json).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    let round = Recording::from_json(&parsed).expect("from_json");
    assert_eq!(round.events.len(), 1);
    match &round.events[0] {
        RecordedEvent::ModelCall { provider, response } => {
            assert_eq!(provider, "counting");
            assert!(response.content.contains("response"));
        }
        _ => panic!("expected ModelCall event"),
    }
}

#[test]
fn replay_exhausted_yields_a_clean_error() {
    let src = r#"
fn main() -> String uses { LLM, Net } {
    let a = ask my_model { user: "q1" }
    let b = ask my_model { user: "q2" }
    a + b
}"#;
    let (_file, program) = load(src);

    // Set up a recording with only one event but the program will issue two.
    let mut rec = Recording::new();
    rec.push(RecordedEvent::ModelCall {
        provider: "x".into(),
        response: ChatResponse {
            content: "only one".into(),
            blocks: vec![ContentBlock::Text("only one".into())],
            ..Default::default()
        },
    });

    let mut interp = Interpreter::with_caps(CapSet::standard_default());
    interp.globals.define(
        "my_model",
        Value::Model(Arc::new(CountingProvider {
            counter: AtomicU32::new(0),
            limit: 0,
        })),
    );
    interp.enable_replay(rec);
    interp.load_program(&program);
    let err = interp.run_main().unwrap_err();
    assert!(
        err.message.contains("replay exhausted"),
        "msg = {}",
        err.message
    );
}

// Suppress unused-import warning in the case where a future refactor drops
// a type below; keeps the imports compact otherwise.
#[allow(dead_code)]
fn _kept(_: Budget, _: Replay) {}
