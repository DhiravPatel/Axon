//! Parser integration tests.
//!
//! Each test parses a representative snippet — most lifted directly from the
//! README — and asserts both that no diagnostics fire and that the resulting
//! AST has the expected top-level shape.

use axon_ast::*;
use axon_diag::SourceFile;
use axon_parser::parse;

fn parse_ok(src: &str) -> Program {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");
    program
}

#[test]
fn empty_program_is_ok() {
    let p = parse_ok("");
    assert!(p.items.is_empty());
}

#[test]
fn use_decl_with_list_and_alias() {
    let p = parse_ok("use std.collections.{HashMap, BTreeSet}\nuse foo.bar as baz");
    assert_eq!(p.items.len(), 2);
    match &p.items[0] {
        Item::Use(u) => {
            assert_eq!(u.path.segments.last().unwrap().name, "collections");
            let items = u.items.as_ref().unwrap();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].name, "HashMap");
        }
        _ => panic!("expected a use decl"),
    }
    match &p.items[1] {
        Item::Use(u) => {
            assert_eq!(u.alias.as_ref().unwrap().name, "baz");
        }
        _ => panic!("expected a use decl"),
    }
}

#[test]
fn function_with_effect_row_and_return_type() {
    let p = parse_ok("pub fn discount(p: Float, price: Float) -> Float uses { LLM, Net } { price }");
    let Item::Fn(f) = &p.items[0] else { panic!() };
    assert!(matches!(f.vis, Visibility::Public));
    assert_eq!(f.name.name, "discount");
    assert_eq!(f.params.len(), 2);
    let row = f.effect_row.as_ref().unwrap();
    assert_eq!(row.effects.len(), 2);
    assert_eq!(row.effects[0].path.segments[0].name, "LLM");
}

#[test]
fn type_record_with_refinement_and_default() {
    let p = parse_ok("pub type User { name: String, age: Int @range(0, 200) = 18 }");
    let Item::Type(t) = &p.items[0] else { panic!() };
    let TypeDeclBody::Record(fields) = &t.body else { panic!() };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[1].refinements[0].name.name, "range");
    assert!(fields[1].default.is_some());
}

#[test]
fn type_sum_with_alternatives() {
    let p = parse_ok("type Shape = Circle(Float) | Square(side: Float) | None");
    let Item::Type(t) = &p.items[0] else { panic!() };
    let TypeDeclBody::Sum(variants) = &t.body else {
        panic!("expected sum, got {:?}", t.body)
    };
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0].name.name, "Circle");
    assert_eq!(variants[2].name.name, "None");
}

#[test]
fn schema_with_version_and_field_refinements() {
    let p = parse_ok(
        "schema Answer @version(2) {\n  text: String @min_len(1)\n  citations: [String]\n}",
    );
    let Item::Schema(s) = &p.items[0] else { panic!() };
    assert_eq!(s.version, Some(2));
    assert_eq!(s.fields.len(), 2);
    assert_eq!(s.fields[0].refinements[0].name.name, "min_len");
}

#[test]
fn agent_with_state_handler_and_effect_row() {
    let p = parse_ok(
        r#"agent Echo(name: String) {
            state count: Int = 0
            on say(msg: String) -> String uses { Console } {
                "echo"
            }
        }"#,
    );
    let Item::Agent(a) = &p.items[0] else { panic!() };
    assert_eq!(a.name.name, "Echo");
    assert_eq!(a.params.len(), 1);
    let mut saw_state = false;
    let mut saw_handler = false;
    for m in &a.members {
        match m {
            AgentMember::State { name, .. } if name.name == "count" => saw_state = true,
            AgentMember::Handler(h) if h.name.name == "say" => saw_handler = true,
            _ => {}
        }
    }
    assert!(saw_state && saw_handler);
}

#[test]
fn match_with_guards_and_or_patterns() {
    let p = parse_ok(
        r#"fn f(x: Int) -> Int {
            match x {
                0 | 1 => 0,
                n if n > 10 => 100,
                _ => x
            }
        }"#,
    );
    let Item::Fn(f) = &p.items[0] else { panic!() };
    // Block tail is the match expression.
    let tail = f.body.tail.as_ref().unwrap();
    let ExprKind::Match { arms, .. } = &*tail.kind else {
        panic!()
    };
    assert_eq!(arms.len(), 3);
    assert!(arms[1].guard.is_some());
}

#[test]
fn pipeline_and_postfix_await_try_force() {
    let p = parse_ok("fn g() { x |> y await? }");
    let Item::Fn(f) = &p.items[0] else { panic!() };
    // We just smoke-test that this parses.
    assert!(f.body.tail.is_some());
}

#[test]
fn generate_with_schema_argument() {
    // `model` and `prompt` are reserved keywords (§9.3), so positional
    // arguments must use non-keyword names. Named-arg *names* may be
    // keywords (`temperature = ...`), but positional *values* may not.
    let p = parse_ok("fn h() { generate<Answer>(brain, my_prompt, temperature = 0.2) }");
    let Item::Fn(f) = &p.items[0] else { panic!() };
    let tail = f.body.tail.as_ref().unwrap();
    let ExprKind::Generate {
        is_gen_shorthand, ..
    } = &*tail.kind
    else {
        panic!()
    };
    assert!(!is_gen_shorthand);
}

#[test]
fn plan_with_prompt_slots() {
    let p = parse_ok(
        r#"fn i() {
            plan with self.model {
                system: "be precise"
                user: question
                tools: [search]
                budget: budget(usd = 0.05)
            }
        }"#,
    );
    let Item::Fn(f) = &p.items[0] else { panic!() };
    let tail = f.body.tail.as_ref().unwrap();
    let ExprKind::Plan { slots, .. } = &*tail.kind else {
        panic!("expected plan, got {:?}", tail.kind)
    };
    assert_eq!(slots.len(), 4);
    assert_eq!(slots[0].label.as_ref().unwrap().name, "system");
    assert_eq!(slots[3].label.as_ref().unwrap().name, "budget");
}

#[test]
fn spawn_and_address_literals() {
    let p = parse_ok("fn k() { let a = spawn Worker(); @alice }");
    let Item::Fn(f) = &p.items[0] else { panic!() };
    // First stmt is a let with spawn on the RHS.
    let Stmt::Let { value, .. } = &f.body.stmts[0] else {
        panic!()
    };
    assert!(matches!(&*value.kind, ExprKind::Spawn(_)));
}

#[test]
fn readme_researcher_example_parses() {
    let src = r#"agent Researcher(model: Model, tools: { search: Tool }, mem: Memory) {
    on ask(question: Tainted<String>) -> Answer uses { LLM, Net, Memory } {
        let ctx = mem.recall(question.text, k = 6) await
        return plan with self.model {
            system: "Answer only from sources found via the search tool. Cite every claim."
            memory: ctx
            user:   question
            tools:  [self.tools.search]
            output: Answer
            budget: budget(usd = 0.05, tokens = 20_000)
        } await
    }
}

fn main() uses { Spawn, LLM, Net, Console } {
    let a = spawn Researcher(model = brain, tools = { search = web_search }, mem = kb)
    print(a.ask("What changed in the EU AI Act in 2025?".tainted()) await)
}"#;
    let p = parse_ok(src);
    assert_eq!(p.items.len(), 2);
    assert!(matches!(p.items[0], Item::Agent(_)));
    assert!(matches!(p.items[1], Item::Fn(_)));
}
