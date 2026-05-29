# Papercuts found while writing a real Axon agent

The advisor's call: *"build one real, ambitious agent in Axon — every language that succeeded had its authors build something real in it first."* So [examples/dogfood_triage/](examples/dogfood_triage/) is a real support-triage bot — fixtures in, classification across 3 models via `flow_parallel_asks`, customer lookup via memory, routing policy with a VIP override, model-drafted auto-replies, full record/replay determinism.

Every issue below was hit *while writing that agent*. The point isn't that Axon is broken — it isn't — but that no spec section reveals these. They're papercuts only the act of building exposes.

The list is roughly ordered by frequency. The highest-frequency items are also the smallest fixes, so this is a high-leverage cleanup pass for a future stage.

---

## P1 — `let mut` doesn't work; you have to remember `var`

```axon
let mut start: Int = 0   // error: expected a pattern
var start = 0            // works
```

Coming from Rust, `let mut` is the first thing every author writes. The current error (`expected a pattern`, pointing at `mut`) doesn't suggest `var`. The fix is a one-line addition to the parser to recognize `let mut x = ...` and emit a `Diagnostic` with a `Fix` that rewrites it to `var x = ...`. That's exactly the shape of fix the [Stage 32 §32.2 axon fix](FEATURES.md) machinery now supports — the diagnostic just needs to exist.

**Severity**: every new author hits this in the first 10 minutes.

## P2 — `prompt` is a reserved keyword, breaking obvious variable names

```axon
let prompt = "You are a..."   // error: expected an expression, got Keyword(Prompt)
let instructions = "You are a..."
```

Same shape as the `model` papercut already documented in the Stage 32 `flow_parallel_asks` field naming (`target:` instead of `model:`). Keywords reserved at the spec level: `prompt`, `model`, `agent`, `tool`, `memory`. All of these are *also* the most natural variable names in an agent codebase. The fix is to make the keywords contextual — only reserved at top-level item-starting positions, not inside expressions.

**Severity**: very high, because the natural prompt-engineering vocabulary collides with the keyword set.

## P3 — Multi-line expression continuation requires explicit parens

```axon
let s = "a " +
        "b "    // error: expected an expression, got Newline

let s = ("a " +
         "b ")  // works
```

Newlines are statement-terminating, and a trailing binary operator on the previous line is *not* a continuation cue. Most agent code wants multi-line string concatenation for prompts; the parens grow tiresome. Either (a) make `+`/`-`/`*` at end-of-line implicitly join, or (b) ship a multi-line raw-string syntax (`"""..."""`) so prompts don't need concat at all.

**Severity**: high. Hit twice in the dogfood agent.

## P4 — No `for` loop over `List<T>`; everything is `while + list_get`

```axon
// what you want:
for x in xs { ... }

// what you have to write:
var i = 0
while i < list_len(xs) {
    let x = list_get(xs, i)
    ...
    i = i + 1
}
```

This is the most visible code-quality cost across the dogfood project. Every fixture loader, every classifier consensus, every audit walk turns into an explicit index loop. The runtime already supports `for_await stream { ... }` for `Stream<T>`; lifting that surface to `for x in list` is small.

**Severity**: high — bloats every list-processing function 3-4×.

## P5 — `mock_model("script", [...])` counter is call-based, not key-based

The drafter in the agent only fires for tickets the policy lets through (3 of 5). The script index has to manually skip the human-review tickets, so the responses don't match the ticket indices:

```axon
mock_model("script", [
    "draft for Alice (ticket 0)",
    "draft for Bob (ticket 1)",
    // ticket 2 is VIP — no draft requested
    "draft for Dave (ticket 3)",
    // ticket 4 is over-threshold refund — no draft requested
])
```

Re-ordering the policy, the fixtures, or which tickets get human-review silently breaks the test. A `MockBehavior::Keyed(Vec<(String, String)>)` indexed by the request's user-text would make scripted mocks robust to caller changes. Or: just let `Turns(...)` cover this; the dogfood project never needed real `Turns`.

**Severity**: medium. Catches you when fixtures change.

## P6 — `axon replay` can't take a project directory

```bash
axon replay rec.json examples/dogfood_triage   # error: Is a directory
axon run --replay rec.json examples/dogfood_triage   # works
```

Two replay paths exist with different argument shapes; only one accepts projects. The single-file `axon replay` predates `LoadedProject`. The fix is one-line: have `axon replay` route the source path through `LoadedProject::load_with_features` when it's a directory.

**Severity**: medium. New authors discover the working command on the second try.

## P7 — Inline record types don't parse inside generics

```axon
fn build_asks(target: Model) -> List<{ target: Model, user: String }> { ... }
//                                   ^ expected `}` to close map/set type
```

The parser tries to read `{ target: Model, user: String }` as a map/set type, not a record type. Even adding explicit field names doesn't help. Workaround is to use `List<dyn>` (P8) or factor the element into a named schema. A real solution: when the next token after `<` is `{`, the parser should commit to record-type parsing.

**Severity**: low. Most production code uses named schemas anyway.

## P8 — `Dyn` / `dyn` is on the type lattice but barely surface-visible

```axon
let xs: List<dyn> = []   // works
let xs: List<Dyn> = []   // error: cannot find type `Dyn` in this scope
```

The Display impl for `Ty::Dyn` produces lowercase `dyn`, and the parser accepts lowercase as a type identifier. But the type checker's registered names use the case-sensitive surface form, so `Dyn` (the Rusty capitalization) is unknown. `axon explain` doesn't document either form. Either pick one and document it, or expose a named `Any` type.

**Severity**: low, but confusing the first time you reach for it.

## P9 — No `Duration → Int` exposed in user code

```axon
let t0 = time_now()
do_work()
let t1 = time_now()
let dt = t1 - t0          // Duration
let ms = dt.as_ms()       // error: no method `as_ms` on type `Duration`
let ms = dt + 0           // error: operator `+` is not defined on `Duration` and `Int`
```

You can subtract two `time_now()` results to get a `Duration`, but you can't ask how long it was. The dogfood agent wanted to print "auto-replied N tickets in M ms" in its summary; couldn't. Same with the benchmark harness — we moved timing entirely to Rust because Axon can't time itself today.

The runtime stores durations as nanoseconds (`Value::Duration(i64)`); a `.as_ms()` / `.as_ns()` method is a 5-line add in [crates/axon-runtime/src/eval.rs](crates/axon-runtime/src/eval.rs).

**Severity**: high for benchmarks; medium for normal agent code.

## P10 — No `String.split`, `String.trim`, `String.replace`

The dogfood agent ships its own `split_pipe` and `split_lines` because the stdlib has `str_substring` and `str_contains` but nothing higher-level. For pipe-delimited fixtures that's tolerable; for any real text munging (CSV, log parsing, prompt-template substitution) you'd be re-implementing the wheel on every codebase.

These are five-line additions; they belong in [crates/axon-std](crates/axon-std/) alongside the existing `str_*` helpers.

**Severity**: medium. Every nontrivial agent codebase hits this.

## P11 — `human_request(...)` opens an approval ticket but there's no in-process gate

`human_request` returns an id you can poll with `human_resolve`. But there's no built-in "block this ticket until the human responds" primitive. For a real triage workflow you'd want something like `await human_decision(id, on_timeout = "deny")`. The dogfood agent works around it by labeling the action `"human_review"` and noting in the audit log that a human is expected to pick up the ticket — we never block.

This is partly a deliberate scope call (the async-runtime migration is the right substrate for real blocking) — but it means the "approval-required" path doesn't actually exercise approval semantics today.

**Severity**: medium. Hides the seam between "agent decided" and "human reviewed".

## P12 — `Audit` cap exists but the audit_* host bindings aren't wired

The agent's `axon.toml` lists `Audit` in the default caps, expecting an `audit_record(...)` host binding. There isn't one — only `local_memory()` does append-only logs, and only `policy_block_audit_summary` exposes anything labeled "audit". So we wrote `audit.store(...)` against a `local_memory` handle and called it the audit trail.

That's good enough for the dogfood demo, but a real triage system would want a tamper-evident audit log keyed by ticket id, with timestamps the runtime injects. The shape exists in the spec; the host binding doesn't.

**Severity**: low — the spec compromise is workable for prototypes.

## P13 — `axon check` reports E0203 twice for one typo

When a type typo (`Profle`) appears in a function signature, the diagnostic emits twice — once during signature lowering and once during body inference. The §32.2 `axon fix` machinery handles this gracefully (the second fix is deferred for overlap), but the user sees the same error rendered twice in `axon check`. The lowering pass should mark types it has already errored on so the body-checking pass skips them.

**Severity**: low (cosmetic), but noticeable to anyone reading diagnostics carefully.

---

## What this list says about Axon

Most of the items above are **5-50 line fixes**. None are architectural. None require revisiting the spec. The papercuts are a function of "the small set of things every new author tries first," and they aggregate fast.

A real Stage 33 — the one *after* the multi-week async migration — would be a UX-focused stage that walks this list top to bottom. P1, P2, P3, P4 alone would dramatically lower the bar for a third-party developer trying Axon for the first time.

That, plus the §32.1 numbers in [BENCHMARKS.md](BENCHMARKS.md), is the credibility loop the advisor pointed at: real agent → real numbers → real friction list → real cleanup stage.
