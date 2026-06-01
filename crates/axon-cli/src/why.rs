//! §34.3 — `axon why <EFFECT> [<path>]`
//!
//! Given an effect name (e.g. `Net`) and a source file (or project
//! directory, or `.` for the current dir), trace every chain of calls
//! from `main` that ultimately requires that effect. Output shape:
//!
//! ```text
//! why Net in main:
//!   main()                  uses { Console, Net }
//!     └─ fetch_html()       uses { Net }
//!          └─ http_fetch()  [built-in requires Net]
//! ```
//!
//! Each printed node is either a user-defined fn (with its inferred
//! row in `{...}`) or a built-in call site (`[built-in requires X]`).
//! Branches whose subtree doesn't carry the effect are pruned so the
//! output stays scannable.
//!
//! Coverage scope (deliberate, v0 — keep in sync with `print_help`):
//!
//! * Top-level user fns + tool bodies — walked via the AST.
//! * Built-in calls — looked up against the type checker's
//!   `builtin_effects_for` table.
//! * §34.6 verification fix M4: `generate { ... }` expressions surface
//!   as `[generate requires LLM, Net]` leaves; `spawn x` expressions
//!   surface as `[spawn requires Spawn]` leaves. These mirror the
//!   effects the tyck unconditionally introduces for those syntactic
//!   forms.
//! * §34.6 verification fix M6: method-call effect detection is a
//!   **name-only heuristic** today. `.store(...)` and `.recall(...)`
//!   are surfaced as `[method requires Memory]` regardless of the
//!   receiver's actual type. If you have a `mylist.store(x)` on a
//!   List, it will be falsely attributed to `Memory`. A real
//!   type-aware dispatch would consult the tyck Ctx — that's a
//!   follow-up. Other method-bearing types (Tainted, List, Map,
//!   Set, Chan) don't introduce effects in the dispatch table today,
//!   so they're true no-ops here.
//! * Calls inside agent handler bodies / actor handler bodies / agent
//!   lifecycle hooks aren't traversed in v0 — their nested fns aren't
//!   yet keyed by `(parent_id, name)` in the inferred-effect table.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::ExitCode;

use axon_ast::{Block, CallArg, Expr, ExprKind, ExprOrBlock, FnDecl, Item, Program, Stmt, ToolBody, ToolDecl};
use axon_diag::SourceFile;
use axon_tyck::Ctx;
use axon_types::ItemSigKind;

/// One node in the call-chain tree we print to stdout.
struct WhyNode {
    label: String,
    children: Vec<WhyNode>,
}

/// Entry point for `axon why <EFFECT> [<path>]`. Loads the project (or
/// single file), runs tyck, walks the call graph, and prints the chain.
pub fn cmd_why(args: &[String]) -> ExitCode {
    let mut effect: Option<&str> = None;
    let mut path: Option<&str> = None;
    let mut entry_name: String = "main".to_string();
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--from" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("axon why: --from requires a fn name (e.g. --from main)");
                    return ExitCode::from(2);
                }
                entry_name = args[i].clone();
            }
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => {
                eprintln!("axon why: unknown flag `{other}`");
                return ExitCode::from(2);
            }
            other => {
                if effect.is_none() {
                    effect = Some(other);
                } else if path.is_none() {
                    path = Some(other);
                } else {
                    eprintln!("axon why: unexpected extra argument `{other}`");
                    return ExitCode::from(2);
                }
            }
        }
        i += 1;
    }
    let Some(effect) = effect else {
        print_help();
        return ExitCode::from(2);
    };
    let path = path.unwrap_or(".");

    // Load via LoadedProject — single-file path also works (it builds
    // a one-module LoadedProject for free).
    let project = match axon_project::LoadedProject::load(Path::new(path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axon why: {e}");
            return ExitCode::from(1);
        }
    };
    if !project.diagnostics.is_empty() {
        // Print only errors, not warnings — typeck-errored programs
        // still have a partial Ctx we can walk.
        for d in &project.diagnostics {
            if matches!(d.severity, axon_diag::Severity::Error) {
                eprintln!("axon why: warning — project has errors; results may be partial");
                break;
            }
        }
    }
    let primary_source = project
        .modules
        .first()
        .map(|m| m.source.clone())
        .unwrap_or_else(|| SourceFile::new("<merged>", String::new()));
    let (ctx, _td) = axon_tyck::check(&primary_source, &project.merged);

    let tree = build_tree(&project.merged, &ctx, &entry_name, effect);
    if let Some(root) = tree {
        println!("why {} in {}:", effect, entry_name);
        print_tree(&root, "");
        ExitCode::SUCCESS
    } else {
        println!("axon why: `{}` is not in {}'s effect graph", effect, entry_name);
        ExitCode::SUCCESS
    }
}

fn print_help() {
    println!(
        "usage: axon why <EFFECT> [<path>] [--from <fn>]\n\
         \n\
         Trace every call site that introduces <EFFECT> into the chosen entry\n\
         function (default `main`). <path> can be a single .ax file or a\n\
         project directory; defaults to `.`.\n\
         \n\
         Leaves:\n\
           - built-in calls (`http_fetch`, `read_file`, …) → `[built-in requires X]`\n\
           - `generate {{ ... }}` blocks → `[generate requires LLM, Net]`\n\
           - `spawn x` expressions → `[spawn requires Spawn]`\n\
           - `.store(...)` / `.recall(...)` method calls → `[method requires Memory]`\n\
             (NOTE: name-only heuristic — same-named methods on non-Memory\n\
             receivers will be falsely attributed.)\n\
         \n\
         v0 scope cuts:\n\
           - calls inside agent handler / actor handler / lifecycle bodies\n\
             are not traversed (the inferred-effect table doesn't key them\n\
             by parent yet).\n\
           - method-effect dispatch is name-only, not type-aware.\n\
         \n\
         Example:\n\
           axon why Net src/main.ax\n\
           axon why Memory ./my-project --from worker\n"
    );
}

// ---------------------------------------------------------------------------
// Tree construction
// ---------------------------------------------------------------------------

/// Build the call-chain subtree rooted at `entry_name` that traces every
/// path that introduces `effect`. Returns `None` when the effect isn't
/// in `entry_name`'s row at all.
fn build_tree(program: &Program, ctx: &Ctx, entry_name: &str, effect: &str) -> Option<WhyNode> {
    // Index every top-level fn / tool body by name. This is the AST
    // walker's lookup table — we use it to recurse into callees.
    let fns: HashMap<&str, &FnDecl> = program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Fn(f) => Some((f.name.name.as_str(), f)),
            _ => None,
        })
        .collect();
    let tools: HashMap<&str, &ToolDecl> = program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Tool(t) => Some((t.name.name.as_str(), t)),
            _ => None,
        })
        .collect();

    let entry = fns
        .get(entry_name)
        .copied()
        .map(BodySource::Fn)
        .or_else(|| tools.get(entry_name).copied().map(BodySource::Tool))?;
    // Quick check: does the entry's row even mention this effect?
    let row = effect_row_for(ctx, entry_name)?;
    if !row.contains(effect) {
        return None;
    }
    let mut visiting: HashSet<&str> = HashSet::new();
    Some(walk_body(
        &entry,
        entry_name,
        effect,
        ctx,
        &fns,
        &tools,
        &mut visiting,
    ))
}

#[derive(Clone, Copy)]
enum BodySource<'a> {
    Fn(&'a FnDecl),
    Tool(&'a ToolDecl),
}

impl<'a> BodySource<'a> {
    fn body(&self) -> Option<&'a Block> {
        match self {
            BodySource::Fn(f) => Some(&f.body),
            BodySource::Tool(t) => match &t.body {
                ToolBody::Block(b) => Some(b),
                ToolBody::Extern { .. } => None,
            },
        }
    }
}

fn walk_body<'a>(
    src: &BodySource<'a>,
    name: &str,
    effect: &str,
    ctx: &Ctx,
    fns: &HashMap<&'a str, &'a FnDecl>,
    tools: &HashMap<&'a str, &'a ToolDecl>,
    visiting: &mut HashSet<&'a str>,
) -> WhyNode {
    let row = effect_row_for(ctx, name);
    let label = format!(
        "{}()  {}",
        name,
        match &row {
            Some(r) => format!("uses {}", format_row_atoms(r)),
            None => "uses { ? }".to_string(),
        }
    );
    let mut children = Vec::new();
    if let Some(body) = src.body() {
        let mut call_sites = Vec::new();
        collect_calls(body, &mut call_sites);
        for call in &call_sites {
            // §34.6 verification fix M4 — synthetic GenerateBlock /
            // SpawnExpr leaves come first, before name-based dispatch.
            match call.kind {
                CallKind::GenerateBlock => {
                    if effect == "LLM" || effect == "Net" {
                        children.push(WhyNode {
                            label: format!(
                                "generate {{ ... }}  [requires LLM, Net — {}]",
                                effect
                            ),
                            children: Vec::new(),
                        });
                    }
                    continue;
                }
                CallKind::SpawnExpr => {
                    if effect == "Spawn" {
                        children.push(WhyNode {
                            label: "spawn ...  [requires Spawn]".to_string(),
                            children: Vec::new(),
                        });
                    }
                    continue;
                }
                CallKind::Regular => {}
            }
            // Recursion guard: don't re-enter a fn that's already on
            // the current path. Records as a leaf with a clear marker.
            if visiting.contains(call.callee.as_str()) {
                children.push(WhyNode {
                    label: format!("{}()  [recursion]", call.callee),
                    children: Vec::new(),
                });
                continue;
            }
            // Built-in introduced effect.
            if let Some(r) = ctx.builtin_effects_for(&call.callee) {
                if r.contains(effect) {
                    children.push(WhyNode {
                        label: format!("{}()  [built-in requires {}]", call.callee, effect),
                        children: Vec::new(),
                    });
                }
                continue;
            }
            // §34.6 verification fix M6 — name-only heuristic. See
            // module rustdoc + print_help for the disclosure.
            if let Some(method) = &call.method {
                if method == "store" || method == "recall" {
                    if effect == "Memory" {
                        children.push(WhyNode {
                            label: format!(
                                "{}.{}()  [method requires Memory — name-only match]",
                                call.callee, method
                            ),
                            children: Vec::new(),
                        });
                    }
                }
                continue;
            }
            // User-defined fn / tool — recurse.
            let next_src = fns
                .get(call.callee.as_str())
                .copied()
                .map(BodySource::Fn)
                .or_else(|| tools.get(call.callee.as_str()).copied().map(BodySource::Tool));
            if let Some(next) = next_src {
                let callee_row = effect_row_for(ctx, &call.callee);
                if callee_row.map(|r| r.contains(effect)).unwrap_or(false) {
                    // Owned key for the visiting-set; the lifetime
                    // tracker requires a borrow with the same lifetime
                    // as `fns`/`tools`, so we look up the interned key.
                    let key: &str = if let Some((k, _)) = fns.get_key_value(call.callee.as_str()) {
                        *k
                    } else if let Some((k, _)) = tools.get_key_value(call.callee.as_str()) {
                        *k
                    } else {
                        continue;
                    };
                    visiting.insert(key);
                    children.push(walk_body(
                        &next,
                        key,
                        effect,
                        ctx,
                        fns,
                        tools,
                        visiting,
                    ));
                    visiting.remove(key);
                }
            }
        }
    }
    WhyNode { label, children }
}

/// Pulls the inferred effect row for `name`, falling back to the
/// declared row when inference didn't run (e.g. parse errors).
fn effect_row_for(ctx: &Ctx, name: &str) -> Option<axon_types::EffectRow> {
    if let Some(r) = ctx.inferred_effects_for(name) {
        return Some(r.clone());
    }
    let id = ctx.lookup(name)?;
    let sig = ctx.get(id)?;
    match &sig.kind {
        ItemSigKind::Fn(fs) | ItemSigKind::Tool(fs) | ItemSigKind::Prompt(fs) => {
            Some(fs.effects.clone())
        }
        _ => None,
    }
}

fn format_row_atoms(r: &axon_types::EffectRow) -> String {
    let mut atoms: Vec<&str> = r.atoms().iter().map(|a| a.name.as_str()).collect();
    atoms.sort();
    if atoms.is_empty() {
        "{ }".to_string()
    } else {
        format!("{{ {} }}", atoms.join(", "))
    }
}

// ---------------------------------------------------------------------------
// AST call-site walker
// ---------------------------------------------------------------------------

struct CallSite {
    /// The callee path's bare name (for `foo(...)`, that's `foo`; for
    /// `mem.recall(...)`, that's `mem` and `method` is `recall`).
    /// Synthetic call sites for `generate { ... }` and `spawn x` use
    /// `kind` to mark themselves; their `callee` is the descriptive
    /// label rendered in the tree (e.g. `"generate"`).
    callee: String,
    method: Option<String>,
    kind: CallKind,
}

#[derive(Clone, Copy)]
enum CallKind {
    /// Plain `foo(...)` call — recurse if it resolves to a user fn,
    /// emit a built-in leaf otherwise.
    Regular,
    /// §34.6 verification fix M4 — `generate { ... }` block; tyck
    /// unconditionally introduces LLM + Net.
    GenerateBlock,
    /// §34.6 verification fix M4 — `spawn x` expression; tyck
    /// unconditionally introduces Spawn.
    SpawnExpr,
}

fn collect_calls(block: &Block, out: &mut Vec<CallSite>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Var { value, .. } => walk_expr(value, out),
            Stmt::Expr(e) => walk_expr(e, out),
        }
    }
    if let Some(t) = &block.tail {
        walk_expr(t, out);
    }
}

fn walk_expr(e: &Expr, out: &mut Vec<CallSite>) {
    match &*e.kind {
        ExprKind::Call { callee, args } => {
            if let ExprKind::Path(p) = &*callee.kind {
                if let Some(seg) = p.segments.last() {
                    out.push(CallSite {
                        callee: seg.name.clone(),
                        method: None,
                        kind: CallKind::Regular,
                    });
                }
            }
            walk_expr(callee, out);
            walk_call_args(args, out);
        }
        ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            // Receiver's bare name is what we record; the method dispatch
            // table keys off the receiver's type, so we approximate by
            // attaching the method name and letting the walker decide.
            if let ExprKind::Path(p) = &*receiver.kind {
                if let Some(seg) = p.segments.last() {
                    out.push(CallSite {
                        callee: seg.name.clone(),
                        method: Some(method.name.clone()),
                        kind: CallKind::Regular,
                    });
                }
            }
            walk_expr(receiver, out);
            walk_call_args(args, out);
        }
        // §34.6 verification fix M4 — `generate { ... }` introduces
        // LLM + Net unconditionally. Emit a synthetic call site so the
        // tree shows the source instead of a misleading empty branch.
        ExprKind::Generate {
            model,
            prompt,
            extra,
            ..
        } => {
            out.push(CallSite {
                callee: "generate".to_string(),
                method: None,
                kind: CallKind::GenerateBlock,
            });
            walk_expr(model, out);
            walk_expr(prompt, out);
            walk_call_args(extra, out);
        }
        ExprKind::Block(b) => collect_calls(b, out),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, out);
            collect_calls(then_branch, out);
            if let Some(eb) = else_branch {
                match eb.as_ref() {
                    ExprOrBlock::Expr(ex) => walk_expr(ex, out),
                    ExprOrBlock::Block(bl) => collect_calls(bl, out),
                }
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, out);
            for arm in arms {
                walk_expr(&arm.body, out);
            }
        }
        ExprKind::When { cond, then_branch } => {
            walk_expr(cond, out);
            collect_calls(then_branch, out);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr(iter, out);
            collect_calls(body, out);
        }
        ExprKind::While { cond, body } => {
            walk_expr(cond, out);
            collect_calls(body, out);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, out);
            walk_expr(rhs, out);
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, out),
        ExprKind::Pipeline { lhs, rhs } => {
            walk_expr(lhs, out);
            walk_expr(rhs, out);
        }
        ExprKind::Return(Some(v)) => walk_expr(v, out),
        // §34.6 verification fix M4 — `spawn x` introduces the `Spawn`
        // effect unconditionally. Emit a leaf BEFORE recursing into
        // the inner expression so `axon why Spawn` has something to
        // attribute the effect to.
        ExprKind::Spawn(inner) => {
            out.push(CallSite {
                callee: "spawn".to_string(),
                method: None,
                kind: CallKind::SpawnExpr,
            });
            walk_expr(inner, out);
        }
        ExprKind::Await(inner)
        | ExprKind::Try(inner)
        | ExprKind::Force(inner) => walk_expr(inner, out),
        ExprKind::TryRecover { body, recover } => {
            collect_calls(body, out);
            walk_expr(&recover.body, out);
        }
        ExprKind::Ask { target, slots } | ExprKind::Plan { target, slots } => {
            walk_expr(target, out);
            for s in slots {
                walk_expr(&s.value, out);
            }
        }
        _ => {}
    }
}

fn walk_call_args(args: &[CallArg], out: &mut Vec<CallSite>) {
    for a in args {
        match a {
            CallArg::Positional(v) | CallArg::Named { value: v, .. } => walk_expr(v, out),
        }
    }
}

// ---------------------------------------------------------------------------
// Tree printer
// ---------------------------------------------------------------------------

fn print_tree(node: &WhyNode, indent: &str) {
    if indent.is_empty() {
        println!("  {}", node.label);
    } else {
        println!("  {}└─ {}", indent, node.label);
    }
    let new_indent = format!("{indent}     ");
    for child in &node.children {
        print_tree(child, &new_indent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(src: &str) -> (Program, Ctx) {
        let file = SourceFile::new("t.ax", src.to_string());
        let (p, diags) = axon_parser::parse(&file);
        assert!(diags.is_empty(), "{diags:#?}");
        let (ctx, _td) = axon_tyck::check(&file, &p);
        (p, ctx)
    }

    #[test]
    fn empty_when_main_doesnt_need_the_effect() {
        let (p, ctx) = analyze("fn main() uses { Console } { print(\"hi\") }\n");
        let t = build_tree(&p, &ctx, "main", "Net");
        assert!(t.is_none(), "Net is not in main's row → no tree");
    }

    #[test]
    fn leaf_builtin_when_main_directly_calls_it() {
        let (p, ctx) = analyze(
            "fn main() uses { Console, Net } { print(http_fetch(\"x\")) }\n",
        );
        let t = build_tree(&p, &ctx, "main", "Net").expect("tree");
        // Children should include the built-in leaf.
        assert!(
            t.children.iter().any(|c| c.label.contains("http_fetch")
                && c.label.contains("built-in requires Net")),
            "tree children: {:#?}",
            t.children.iter().map(|c| &c.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_level_chain_through_helper() {
        let (p, ctx) = analyze(
            "fn helper() uses { Net } { http_fetch(\"x\") }\n\
             fn main() uses { Net } { helper() }\n",
        );
        let t = build_tree(&p, &ctx, "main", "Net").expect("tree");
        assert!(t.label.starts_with("main()"));
        assert_eq!(t.children.len(), 1, "{:#?}", t.children.iter().map(|c| &c.label).collect::<Vec<_>>());
        let helper = &t.children[0];
        assert!(helper.label.starts_with("helper()"), "{}", helper.label);
        assert_eq!(helper.children.len(), 1);
        assert!(helper.children[0].label.contains("http_fetch"));
    }

    #[test]
    fn branch_pruning_drops_unrelated_callees() {
        let (p, ctx) = analyze(
            "fn other() { }\n\
             fn fetch() uses { Net } { http_fetch(\"x\") }\n\
             fn main() uses { Net } { other()\n  fetch() }\n",
        );
        let t = build_tree(&p, &ctx, "main", "Net").expect("tree");
        // `other()` doesn't carry Net — must be pruned.
        assert!(t.children.iter().all(|c| !c.label.starts_with("other()")));
        // `fetch()` carries Net — must appear.
        assert!(t.children.iter().any(|c| c.label.starts_with("fetch()")));
    }

    #[test]
    fn recursion_marked_as_leaf_not_infinite_loop() {
        // Mutual recursion: a → b → a. The cycle must terminate via the
        // [recursion] marker, not stack-overflow.
        let (p, ctx) = analyze(
            "fn a() uses { Net } { b() }\n\
             fn b() uses { Net } { a()\n  http_fetch(\"x\") }\n\
             fn main() uses { Net } { a() }\n",
        );
        let t = build_tree(&p, &ctx, "main", "Net").expect("tree");
        // Walk the tree looking for a [recursion] marker on either name.
        let mut stack = vec![&t];
        let mut found_recursion = false;
        while let Some(n) = stack.pop() {
            if n.label.contains("[recursion]") {
                found_recursion = true;
            }
            for c in &n.children {
                stack.push(c);
            }
        }
        assert!(found_recursion, "expected a [recursion] leaf");
    }
}
