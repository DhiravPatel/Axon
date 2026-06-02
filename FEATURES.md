# Axon — Implemented Features

A snapshot of everything Axon ships today, grouped by the stages that introduced each capability. All features below are covered by the workspace test suite (**1052 tests passing** across 30+ crates).

---

## Stage 35 — Authoring Ergonomics: Native Agent Slots + `axon watch` + Trajectory Snapshots + Doc Tests + Project-Mode Interactive Fix

The advisor's "authoring layer" priorities, shipped end-to-end. Async eval core is **explicitly still deferred** — every prior advisor (Stage 32/33/34) flagged it as multi-week, and we keep faithfully delivering bounded ergonomic improvements that compound while the substrate matures. Adversarial verification surfaced 3 critical + 8 major findings; all addressed before merge.

### §35.1 Native agent declaration slots — `uses_tools` / `memory` / `policy` / `strategy`

The advisor's "collapses authoring ceremony" win. The parser already accepted arbitrary `key: value` members inside an `agent { ... }` block; Stage 35 makes four well-known keys semantic slots that desugar at spawn time.

- **`agent X { uses_tools: [a, b], memory: local_memory(), policy: name, strategy: "ReAct", on … {} }`** — handler bodies reach the values via `self.tools` / `self.memory` / `self.policy` / `self.strategy` (canonical runtime names).
- **Tyck side** ([crates/axon-tyck/src/register.rs](crates/axon-tyck/src/register.rs)) — `state_field_sigs` injects virtual fields per slot so `self.<slot>` typechecks: `tools: List<dyn>`, `memory: Memory`, `policy: String`, `strategy: String`. **§35.6 verification fix M1** — slot expressions are typechecked against the canonical expected type (`memory: 42` is now an E0211, not a runtime crash).
- **Runtime side** ([crates/axon-runtime/src/eval.rs](crates/axon-runtime/src/eval.rs)) — `eval_spawn` walks `def.settings`, recognizes the four canonical keys, evaluates each in `init_env`, and pushes onto state under the runtime name. **§35.6 verification fix C1** — the user's explicit `state <name> = ...` field wins over a same-named slot (precedence rule, runtime and tyck now agree).
- **v0 disclosures** (in module rustdoc + example header): `uses_tools` is `List<dyn>` not `List<Tool>`; `policy: ident` and `strategy: ident` are stored as `String` of the ident's text (no resolution to a `policy <name> { ... }` block — `policy_block_check` consumes a string name, so this matches the consumer).

### §35.2 `axon watch <file>` — live trace inspector

The advisor's "observe instead of re-run with prints" priority.

- **New streaming sink** ([crates/axon-runtime/src/trace.rs](crates/axon-runtime/src/trace.rs)) — `pub type StreamSink = Box<dyn FnMut(&TraceSpan) + Send>`. `Tracer::close()` invokes the sink with a clone of the just-closed span. `with_sink` / `set_sink` builder API; spans still buffer in `spans` so end-of-run `--trace PATH` flush works.
- **`Interpreter::enable_tracing_streaming(sink)`** ([crates/axon-runtime/src/eval.rs](crates/axon-runtime/src/eval.rs)).
- **CLI** ([crates/axon-cli/src/main.rs](crates/axon-cli/src/main.rs) `cmd_watch` + new [crates/axon-cli/src/watch_format.rs](crates/axon-cli/src/watch_format.rs) module) — `axon watch <file> [--trace PATH] [--no-color]`. One line per closed span to stderr; stdout left alone for program output. Color auto-detected when stderr is a TTY. **§35.6 verification fix M4** — `ctrlc` handler installed so `--trace PATH` flushes on natural exit; banner honestly says SIGINT may abort before flush.
- **`watch_format` is a pure module** with 6 unit tests covering anchor offset, attrs (`model=`, `tokens=`, `cost_usd=`), error markers, and color/no-color.

### §35.3 `axon fix --interactive` for project mode

Closes the Stage 34 "deferred to single-file" gap.

- [crates/axon-cli/src/main.rs](crates/axon-cli/src/main.rs) `cmd_fix_project` takes a new `interactive: bool` param; the per-hunk `prompt_interactive` helper is reused unchanged; `[a]ll-from-here` accepts across all remaining files; `[q]uit` aborts the whole pass; non-TTY stdin falls back to dry-run.
- The 3-tuple `(description, code, edit)` was promoted to a named `ProjectHunk` struct so `confidence` plumbing is index-shuffling-free.

### §35.4 `axon test --record-trajectory` / `--match-trajectory` — snapshot testing for agents

The advisor's "stops examples from rotting" via shape-not-text snapshot comparison.

- **`TrajectoryEvent` + `trajectory_from_events` + `TrajectoryTolerance` + `compare_trajectories`** ([crates/axon-eval/src/trajectory.rs](crates/axon-eval/src/trajectory.rs)) — the new comparison surface. Per-metric tolerances (`step_pct = 0.20`, `grounded_tolerance = 0.10`, `tool_accuracy_tolerance = 0.10`); booleans (`recovered_from_errors`) demand exact equality; allowed-tools is set equality.
- **`cmd_test` wires the per-test loop** to enable recording when either flag is set, builds a `Trajectory` from the recording's `ModelCall` events, and either writes to `tests/.trajectories/<NAME>/<test>.json` or compares against the saved baseline. Drift exits non-zero so CI catches it.
- **§35.6 verification fix C2** — a test whose model calls regress to ZERO is the exact thing snapshot testing is designed to catch; the original code silently passed. Now: empty events still build a trajectory, match always reads the baseline, missing baseline in match mode counts as drift (exits non-zero).
- **§35.6 verification fix M2** — `--record-trajectory <name>` rejects path traversal (`../`, absolute paths, anything that doesn't survive `sanitize_filename` unchanged).
- **Closure-effect change** in `cmd_test`: tests now inherit the runner's full cap set (the manifest's `[caps] default = [...]`) instead of attenuating to empty. Without this, no effectful test could run; the prior `Some(Vec::new())` made the trajectory feature untestable.

### §35.5 `axon test --doc` / `--doc-only` — doc tests for `///` comments

The advisor's "doc generator + test runner both exist; extracting `` ```axon `` blocks is the missing pipe."

- **New module** [crates/axon-doc/src/doctest.rs](crates/axon-doc/src/doctest.rs) — `scan_fences` finds `` ```axon[,flag] `` … `` ``` `` blocks; `classify_info` parses `ignore` / `no_run` flags; `extract_from_project` walks every module's doc pairs and produces `DocSnippet { item_name, body, disposition }`.
- **`synthesize_with_existing`** wraps each non-ignored snippet as `test "doc(item_name)" { <body> }`. `no_run` snippets wrap in `if false { ... }` so they parse + typecheck but never execute. Name dedup against intra-doc collisions (`#2`, `#3`, …).
- **§35.6 verification fix C3** — `body_braces_are_balanced` runs before synthesis; an unbalanced body emits a failing test stub (`assert(false, "doc snippet … has unbalanced braces — splice refused")`) instead of letting a `}` break out of the test wrapper and inject top-level items (tools, fns, tests). Tracks `"` to avoid false positives inside string literals.
- **§35.6 verification fix M3** — `synthesize_with_existing` also dedupes against the user's existing test names so a `///`-attached snippet on `fn add` plus a user-written `test "doc(add)"` never produces an E0204 pointing at the phantom `<doc-tests>` file.
- **`cmd_test --doc`** parses the synthesized source via the project's source registry (so spans attribute back to `<doc-tests>`), splices the synthesized `TestDecl`s into `project.merged.items`, runs everything through the regular test loop. `--doc-only` drops the user's tests first.

### §35.6 — Adversarial verification round: 3 critical + 8 major findings, all fixed

The Stage 35 implementation was adversarially reviewed by a 4-reviewer workflow (correctness/edge-cases, integration/cross-cutting, documentation/honesty, security). Verdict: **fix-then-ship**. All addressed; pinned by regression tests in [crates/axon-cli/tests/stage35_verify_fixes.rs](crates/axon-cli/tests/stage35_verify_fixes.rs):

**Critical:**
- **C1** — slot precedence vs `state` fields inverted (runtime said slot wins, tyck said state wins). Fix: pre-scan `def.state_fields`, skip slots whose canonical name will be claimed. Both layers now agree.
- **C2** — `--match-trajectory` silently passed regressions that removed all model calls. Fix: always read baseline; missing baseline counts as drift; both exit non-zero.
- **C3** — doc-test synthesis spliced raw fence bodies between `test "..." {` and `}` with no balanced-brace check; an unmatched `}` could inject arbitrary top-level items. Fix: `body_braces_are_balanced` (string-literal-aware) refuses unbalanced bodies; emits a clear failing stub.

**Major (must-fix-before-merge):**
- **M1** — slot expressions (`memory: 42`) were never typechecked. Fix: typecheck against the canonical expected type at agent-body check time.
- **M2** — `--record-trajectory <name>` joined `name` raw onto the snapshot dir; `../../etc` escaped the project. Fix: `sanitize_filename` applied to the set name, rejected if it changes.
- **M3** — doc-test name collisions with user tests produced E0204 errors pointing at `<doc-tests>`. Fix: `synthesize_with_existing` suffixes against user test names.
- **M4** — `cmd_watch` banner promised `Ctrl-C to stop` but installed no handler. Fix: `ctrlc::set_handler` installed; banner honestly says SIGINT may abort before `--trace` flush.
- **M5** — `axon test --help` didn't exist; new flags were undiscoverable. Fix: explicit `--help` branch lists every flag.
- **M6** — Stage 35 test `user_state_field_wins_over_same_named_slot_default` didn't actually exercise the conflict path (no `uses_tools` slot in the fixture). Fix: fixture rewritten to declare both; rename the no-conflict sanity test.
- **M7** — `watch_format` header columns didn't align with row output. Fix: padded `kind_label:<10`, `name:<35`, `dur:>6ms`; banner header re-aligned.
- **M8** — `examples/stage35_agent_slots.ax` header under-disclosed v0 looseness; `policy: support_policy` referenced an undefined identifier. Fix: explicit `// v0 cuts:` block in the header explaining `uses_tools: List<dyn>` and that `policy`/`strategy` are stored as strings.

### Test coverage

| Suite | Tests | Pins |
| --- | --- | --- |
| `axon-eval::trajectory` | (existing + new compare/from_events) | tolerance shapes |
| `axon-doc::doctest` | 5 new unit tests | fence scan, flags, synthesis, dedup |
| `axon-lsp::watch_format` | 6 new unit tests | offset, attrs, error marker, color |
| `axon-cli::stage35_agent_slots` | 7 integration tests | each of 4 slots + compose + user-state-only + user-state-wins-over-slot |
| `axon-cli::stage35_watch` | 5 integration tests | startup, stdout split, --trace JSONL, --help, unknown-flag |
| `axon-cli::stage35_project_interactive` | 3 integration tests | non-TTY fallback, --apply round-trip, tier labels |
| `axon-cli::stage35_trajectory` | 5 integration tests | record-then-match round-trip, missing-baseline note, mutex, no-model-skip, shape-drift |
| `axon-cli::stage35_doc_tests` | 6 integration tests | extract, --doc-only, ignore, no_run, parse error, no-fences message |
| `axon-cli::stage35_verify_fixes` | 8 regression tests | C1 slot precedence, C2 zero-call drift, C2 missing baseline, C3 brace injection, M1 type error, M2 path traversal, M3 collision suffix, M5 --help |
| **Workspace total** | **1052 passing**, up from 1007 | |

### What's NOT in this stage (explicit, fifth defer)

- **Async eval core** — Stage 31/32/33/34/35 advisors all flagged this as the priority but multi-week. The substrate (Arc/Send+Sync from Stage 32, `axon-async` from Stage 31, `flow_parallel_asks` proven slice) is settled. The migration of `eval.rs` (~3500 LOC of intricate state) needs 2–3 stages of focused work, not one — trying to do it end-to-end in a session ships broken code.
- **`select` / `parallel { ... }` / `for await` made truly async** — depends on async eval core.
- **Real package registry / git deps / lockfile** — its own stage.
- **GBNF → llama.cpp/vllm wiring** — per-backend bridging, its own stage.
- **Debugger DAP, snapshot testing for non-trajectory shapes, `axon bench`** — each its own stage.

---

## Stage 34 — Errors Heal Themselves In Your Editor: LSP Code Actions + `axon fix --watch` + Effect-Row Lens + `axon why`

The advisor's #2 priority shipped end-to-end, with verification: an adversarial 4-reviewer workflow demanded 3 critical + 7 major fixes before merge. All addressed; this entry documents both what shipped and what the verification pass caught.

### §34.1 Fix confidence tiers (`Confidence::{Safe, Suggested}`)

Foundation for everything else in this stage. Without tiers, `--watch` couldn't safely auto-apply anything and LSP code actions couldn't tell editors which fix to auto-pick.

- **`Confidence` enum on `axon_diag::Fix`** ([crates/axon-diag/src/lib.rs](crates/axon-diag/src/lib.rs)). Default `Suggested` for backwards compat; opt-in `Safe` via `.safe()` builder.
- **Categorization rule (documented + locked by integration tests):**
  - **Safe** — purely additive or deterministic-from-known-set: E0202/E0203/E0207 did-you-mean rename, E0204 `{name}_N` deterministic rename, E0210 effect-row append/synthesize (purely additive — tyck already proved the effects are needed), P0010 `pub` insertion.
  - **Suggested** — deletes user code or inserts placeholders: E0205 nil-padding, E0205 drop-trailing-args, E0205 drop-all-args.
- **8 categorization-locking integration tests** ([crates/axon-cli/tests/stage34_confidence_tiers.rs](crates/axon-cli/tests/stage34_confidence_tiers.rs)) — if a future contributor adds a fix site without explicit tagging, this suite catches the missing `.safe()` on any new Safe-class fix.

### §34.2 Effect-row code lens (LSP)

Surfaces the **inferred** `uses { ... }` row above every top-level `fn` and tool body — the §59 effect-overlay feature the spec promises.

- **Tyck-side: inferred-effect capture** ([crates/axon-tyck/src/ctx.rs](crates/axon-tyck/src/ctx.rs), [crates/axon-tyck/src/infer.rs](crates/axon-tyck/src/infer.rs)) — new `inferred_fn_effects: HashMap<ItemId, EffectRow>` table populated by a new `with_effect_row_capturing` variant that returns the accumulated `used` row. Clobbering risk on duplicate-definition defended by `sig.span == fn.span` guard.
- **LSP-side: new [crates/axon-lsp/src/effect_lens.rs](crates/axon-lsp/src/effect_lens.rs)** — mirrors `cost_lens.rs`. Three statuses: `Derived` (no row written), `Matches` (declared = inferred), `OverDeclared { unused }` (declared is a strict superset; label lists unused atoms).
- Wired into the existing `CodeLensRequest` handler ([crates/axon-lsp/src/server.rs](crates/axon-lsp/src/server.rs)) alongside cost lenses.
- **Honest scope cut (v0):** top-level `Item::Fn` + `Item::Tool` only. Agent member fns share the global name table; nested-fn lenses land when items are keyed by `(parent_id, name)`.

### §34.3 `axon why <EFFECT> [<path>] [--from <fn>]`

Traces every chain of calls from the entry function that ultimately requires `<EFFECT>`:

```
why Net in main:
  main()  uses { Console, Net }
       └─ fetch_html()  uses { Net }
            └─ http_fetch()  [built-in requires Net]
```

- **New [crates/axon-cli/src/why.rs](crates/axon-cli/src/why.rs)** — DFS over the call graph, pruning branches whose subtree doesn't carry the target effect. Mutual recursion guarded with a `[recursion]` leaf.
- Built-in effects via `Ctx::builtin_effects_for`; user fns recurse via the inferred-effect table.
- **§34.6 fix M4** — `generate { ... }` and `spawn x` emit synthetic `[requires LLM, Net]` / `[requires Spawn]` leaves so the tree never has an empty-children dead end.
- **Honest scope cuts (v0, documented in `--help` and module rustdoc):**
  - Method-effect dispatch is **name-only** — `.store(...)` / `.recall(...)` surface as `[method requires Memory — name-only match]` regardless of receiver type.
  - Calls inside agent handler / actor handler / lifecycle bodies aren't traversed.

### §34.4 LSP `textDocument/codeAction` — every diagnostic with a `Fix` gets a lightbulb

The advisor's "where users actually want fixes." Every diagnostic at the cursor that carries a `Fix` shows up as a `CodeAction { kind: QUICKFIX }` with a `WorkspaceEdit` the editor can apply in one click.

- **New [crates/axon-lsp/src/code_actions.rs](crates/axon-lsp/src/code_actions.rs)** — pure function `code_actions_for(analysis, text, uri, range) -> Vec<CodeActionOrCommand>`. One CodeAction per Fix, with the diagnostic linked back so editors dim the squiggle on apply.
- **Safe-tier fixes set `is_preferred = true`** — editors mark them as the auto-pick. "Fix on save" only triggers when the user has explicitly opted in via editor config.
- **Cross-file fixes (P0010, where the edit lives in a different file) are dropped silently in v0** — single-file LSP can't represent the cross-file edit; the CLI's project mode still handles them.
- Server wired: `ServerCapabilities.code_action_provider` advertises QUICKFIX.

### §34.5 `axon fix --interactive` and `axon fix --watch`

- **`--interactive`** — walks each accepted hunk and prompts `[y]es / [n]o / [a]ll-from-here / [q]uit`. Falls back to dry-run cleanly when stdin is not a TTY. Inline preview shows one before/after line of context.
- **`--watch`** — re-runs on every save via [notify](https://crates.io/crates/notify). **Auto-applies only Safe-tier fixes**; Suggested ones report to stdout with a hint to run `axon fix --interactive`. Cooperative shutdown via [ctrlc](https://crates.io/crates/ctrlc).
- **Mutual exclusivity:** `--apply`, `--interactive`, `--watch` are checked exclusive — picking two is exit-2.
- **Tier label in dry-run output:** `fix [E0202, safe]: ...` — a user reviewing the diff knows what would auto-apply under `--watch`.

### §34.6 — Adversarial verification: 3 critical + 7 major findings, all fixed

The Stage 34 implementation was adversarially reviewed. The reviewers' verdict was **block** — production code can't ship with the bugs they found. Every finding was addressed and pinned by regression tests in [crates/axon-cli/tests/stage34_verify_fixes.rs](crates/axon-cli/tests/stage34_verify_fixes.rs):

**Critical (security/data-loss):**

- **C1 — `axon fix --watch <dir>` could rewrite arbitrary system .ax files.** Original code accepted any path that `exists()`. Fixed: directory mode requires an `axon.toml` in the tree (or an ancestor of it) AND requires the canonical watch root to be a descendant of CWD. Override with `AXON_FIX_WATCH_FORCE=1`. Refusal messages name the exact safety rule that fired. Single-file mode is bounded by definition (one file) and isn't subject to the CWD-descendant gate.
- **C2 — Path traversal via `[run] src = "../etc"` and src/ symlinks.** Fixed: `load_directory` canonicalizes the project root + requires `run.src` and each dep's `src/` to canonicalize as descendants of their respective roots. `walk_dir` rejects symlinks via `symlink_metadata` and double-checks each entry against the canonicalized root. Dep paths to sibling project dirs (legitimate workspace pattern) remain allowed.
- **C3 — TOCTOU race between watcher event and apply.** Fixed: stat the file at read, re-stat right before write; abort the apply if mtime advanced.

**Major (correctness/honesty):**

- **M1 — Project-mode `--watch` never recorded mtime per file** → infinite re-trigger loop risk. Fixed: `apply_safe_fixes_to_project` returns the list of touched paths; the watch loop records mtimes for both modes.
- **M2 — Single-file `--watch foo.ax` fired on sibling .ax files** despite the comment claiming a filter. Fixed: canonicalized target path captured before the loop; other-path events dropped.
- **M3 — Watch debounce had no maxWait.** A chatty auto-save editor could slide the deadline forever. Fixed: quiet-window (250 ms) AND maxWait (2 s) — whichever fires first.
- **M4 — `axon why` was missing `ExprKind::Generate` and `ExprKind::Spawn` arms** → false dead-end. Fixed: both emit synthetic call-site leaves.
- **M5 — `effect_lens.rs` rustdoc lied about `OverDeclared` label format and row-variable behavior.** Fixed: rustdoc rewritten to match output; row-variable handling honestly documented as v0 limitation.
- **M6 — `axon why` method dispatch is name-only** — original rustdoc overstated fidelity. Fixed: tree label now reads `[method requires Memory — name-only match]`; `--help` lists it as a v0 scope cut.
- **M7 — Ctrl-C handler installed AFTER `watcher.watch()`** (race window) AND install failure was non-fatal. Fixed: handler installed first; failure is fatal.

### Test coverage

| Suite | Tests | Pins |
| --- | --- | --- |
| `axon-diag::tests` | 2 new | Confidence default + builder semantics |
| `axon-lsp::effect_lens` | 7 new | derived/matches/over-declared/transitive/anchor-span |
| `axon-lsp::code_actions` | 7 new | lightbulb appears, no-fix-no-action, range filter, multiple fixes, is_preferred Safe, cross-file dropped |
| `axon-cli::tests::stage34_confidence_tiers` | 8 new | Every existing fix code's tier locked |
| `axon-cli::tests::stage34_why_and_lens` | 8 new | CLI `axon why` end-to-end + effect-lens smoke |
| `axon-cli::tests::stage34_interactive_watch` | 6 new | Dry-run tier labels, --interactive non-TTY fallback, mode mutex, --watch startup, suggested-not-auto-applied, P0010 cross-file routing |
| `axon-cli::tests::stage34_verify_fixes` | 9 new | C1 manifest gate, C1 CWD gate, C2 `run.src` escape, C2 symlink skip, M2 sibling filter, M4 Generate leaf, M4 Spawn leaf, M6 help honesty |
| **Workspace total** | **1007 passing**, up from 955 | |

### What's NOT in this stage (deliberate)

- **Async eval core** — multi-week per the advisor's own prior note; the bounded `flow_parallel_asks` slice shipped Stage 32. The full migration is its own stage.
- **`axon fix --interactive` for project mode** — per-hunk prompts across many files need per-file UI; single-file ships today.
- **Real package registry / git deps / lockfile** — its own stage.
- **`axon test --doc`** — separate substantial pipeline.
- **Snapshot testing, debugger DAP, `axon bench`** — each a stage-sized commitment.

---

## Stage 33 — `axon fix` Goes Wide + Project Mode + LSP Cost Lens + Footer Loop Closed

Three priority-1/priority-2 advisor items, shipped end-to-end. The pitch: errors are now expected to come with rewrites attached, the rewrites work across a whole project, and the LSP shows the dollar cost of every model call as you type.

### §33.1 Three new mechanical fixes (catalogue: 3 → 6)

The Stage 32 `axon fix` engine proved the apply loop is safe. Adding new fixes is now just a `*_with_fix` constructor in [crates/axon-tyck/src/errors.rs](crates/axon-tyck/src/errors.rs) + a one-line wire at the diagnostic call site.

- **E0204** (duplicate definition) — `duplicate_definition_with_fix(span, name, prev, name_span, existing_names)` picks the first unused `{name}_N` (N = 2..) and replaces the *second* item's name identifier. Existing names are scraped from `Ctx::item_names()` so the rename doesn't collide with any other top-level identifier in scope.
- **E0205** (wrong arity) — `wrong_arity_with_fix(call_span, name, expected, found, arg_spans)`:
  - Too few args → inserts `, nil, nil, ...` right before the closing `)`. Prepended comma when there's at least one existing arg; bare list when none.
  - Too many args → replaces from the end of the last-kept arg to the end of the last actual arg with the empty string, sweeping commas + trailing args in one splice.
  - `expected == 0` (call expects zero, user passed any) special-case → deletes from the first arg's start to the last arg's end so `f(1, 2, 3)` becomes `f()`.
- **E0207** (no such method) — `no_such_method_with_candidates(span, method, on_ty, candidates)`. Candidates come from `handlers` on agent/actor handles, or from a hand-curated `builtin_methods_for(ty)` table mirroring the dispatch arms in `method_call_ty` (kept hand-curated so future entries don't silently change the suggestion surface).
- **`closest()` extended with prefix-of acceptance** — Stage 32 capped Levenshtein at `max(2, len/3).min(4)`. That misses the common typo class `length` → `len` (edit distance 3). The new rule: candidates that are a strict prefix of the target (or vice versa) — with both at least 3 chars long — bypass the edit-distance cap. Catches `length`↔`len`, `recall_all`↔`recall`, `from_string`↔`from_str`.

### §33.2 `axon fix` recursive over a project tree (cross-file fixes route correctly)

- New `cmd_fix_project(root, apply, only)` in [crates/axon-cli/src/main.rs](crates/axon-cli/src/main.rs) — when the argument to `axon fix` is a directory, the CLI loads the project via `LoadedProject::load(root)` (every `.ax` file gets a stable file_id), type-checks the merged program, and routes every fix edit to the right file via `Span::file`.
- **Cross-file P0010 fix.** [crates/axon-project/src/lib.rs](crates/axon-project/src/lib.rs) now stores `(is_pub, item_span)` per export. The P0010 diagnostic fires in the *importing* file but the attached fix targets the *exporting* file — the fix-edit's `Span::file` points to the helper module, and `axon fix` applies it there. Demoed: `use helpers.{greet}` triggers P0010 in `main.ax`; `axon fix --apply` inserts `pub` in `helpers.ax`.
- **Conflict + offset handling unchanged** from Stage 32 — accepted hunks splice in descending start-offset order per file, so an early growth-fix doesn't invalidate later offsets.
- Per-file diff/apply with a summary that names every touched file.

### §33.3 LSP code lens — per-call cost estimate above `ask`/`generate`/`plan`

New crate module [crates/axon-lsp/src/cost_lens.rs](crates/axon-lsp/src/cost_lens.rs). Walks the program AST to find every `ask`/`generate`/`plan` and emits a `CostLens { span, label, input_tokens, assumed_output_tokens, estimated_cost_usd, estimated_latency_ms }` per call:

```
~ $0.0192 · ~9.0s · in 6 / out 256  (ask)
```

- **No API call.** Estimates come from prompt source-text length divided by 4 (the cross-provider ballpark) — same heuristic the Stage 31 cost ledger falls back to. Pessimistic 256-token assumed completion so users budget on the safe side.
- **Provider rates hard-coded** at Opus-tier defaults (`$15/M in`, `$75/M out`, ~30 tok/sec, 600ms setup) — same rates the in-process ledger uses. Exact numbers come from `axon prof --cost` against a real recording.
- **Wired into LSP via `textDocument/codeLens`** — [crates/axon-lsp/src/server.rs](crates/axon-lsp/src/server.rs) advertises `code_lens_provider: Some(CodeLensOptions { resolve_provider: false })` and handles `CodeLensRequest::METHOD`. Editors render the title as an inline label above the call; the command field is empty (clicking is a no-op — by design; it's an info display).
- **Walker covers every relevant nesting** — let/var inits, if/match/while/for bodies, method call receivers, binary operands, pipelines, try/recover bodies, spawn/await/try/force wrappers. New unit tests in `cost_lens` cover the no-asks case, one-lens-per-call counting, and span anchoring.

### §33.4 Mock-model footer — close the loop

The advisor flagged this directly: when `default_model()` falls back to the mock provider because `ANTHROPIC_API_KEY` isn't set, the post-run footer should *say so* on one line. Now it does.

- [crates/axon-runtime/src/builtin.rs](crates/axon-runtime/src/builtin.rs) — new thread-local `DEFAULT_MODEL_FELL_BACK_TO_MOCK: Cell<bool>` flipped to `true` whenever `builtin_default_model` resolves to the mock path. New `default_model_used_mock()` / `reset_default_model_mock_flag()` exports.
- The `axon run` footer in [crates/axon-cli/src/main.rs](crates/axon-cli/src/main.rs) checks the flag right after the cost/tokens line and renders:
  ```
  ─── axon run: 0.21s  ·  0 tokens  ·  $0.0000
      (no ANTHROPIC_API_KEY — `default_model()` used the mock. run `axon login anthropic` to use the real model.)
  ```
- Subject to the same TTY check as the main footer (suppressed in CI / piped output / `AXON_NO_FOOTER`).

### Test coverage
- 4 new unit tests in `crates/axon-lsp::cost_lens` (one-lens-per-ask, nested visit through let-init, no-asks-no-lenses, cost/latency components).
- 10 new end-to-end tests in [crates/axon-cli/tests/stage33_fixes_and_lens.rs](crates/axon-cli/tests/stage33_fixes_and_lens.rs): E0204 rename, E0205 too-few (nil padding), E0205 too-many (drop trailing), E0207 length→len, P0010 cross-file routing, multi-file dry-run summary, cost-lens span anchoring + count, footer-shows-when-mock, footer-silent-when-not.
- Workspace total: **955 passing**, up from 939.

### Honest scope: deferred to a later stage
- **`--watch` mode** for `axon fix --all` — needs `notify` (or similar) for file events; the recursive walk is in place but reruns are manual.
- **More LSP affordances** the spec promises (§59): effect-row code lens, prompt-render preview panel, one-click "record cassette for this test". The cost lens is the highest-leverage one of the four; the others are separate days of work.
- **Doc tests** (`axon test --doc` from §60.2) — separate substantial work; the doc generator and test runner exist but extracting executable ````axon` blocks from `///` comments is its own pipeline.
- **`axon why <effect> <file>`** — useful but a separate command.
- **`axon explain <paste>`** — the explain catalogue already serves this; the convenience flag is a wrapper.
- **`select` / `parallel { ... }` async slices** — these are Stage 34 territory; the Arc-Send-Sync substrate Stage 32 shipped is the foundation, but the per-primitive migration is its own bounded slice each.

---

## Stage 32 — Async I/O Slice: `flow_parallel_asks` + Mechanically Applicable Fixes (`axon fix`)

The first real piece of the async-runtime migration plus the diagnostic-rewrite machinery `axon fix --all` was waiting on. Both are bounded, production-shaped slices — not stage-wide claims.

### §32.1 Async I/O slice — `flow_parallel_asks`

The migration of the whole interpreter to `async fn` is genuinely a multi-week project (every host binding, every channel, every model call). This stage delivers the *bounded first slice*: the boundary where the latency actually lives — model I/O.

- **Trait tightened to `Send + Sync`** — [crates/axon-models/src/lib.rs](crates/axon-models/src/lib.rs): `ModelProvider` now requires `Send + Sync`, so providers can cross thread boundaries. Both shipped impls (`AnthropicProvider`, `MockProvider`) were already thread-safe; the bound just makes it sound.
- **`Value::Model` switched `Rc` → `Arc`** — [crates/axon-runtime/src/value.rs](crates/axon-runtime/src/value.rs): provider handles are now `Arc<dyn ModelProvider>` so a clone can move into a `tokio::spawn_blocking` closure. Touches every Value::Model site: `eval.rs`, `builtin.rs`, the runtime tests, the tools tests.
- **New host binding `flow_parallel_asks(asks)`** — [crates/axon-cli/src/host.rs](crates/axon-cli/src/host.rs): takes a `List<{ target, user, system?, max_tokens? }>`, runs each ask on a `tokio::spawn_blocking` worker, joins in **input order** (not completion order), returns a `List<String>`. The acceptance test proves real overlap: 3 × 200 ms mock asks finish in < 400 ms wall time.
- **Singleton tokio runtime** — held in a `OnceLock<tokio::runtime::Runtime>` with a 4-thread worker pool. `spawn_blocking` uses the default 512-thread blocking pool for the actual provider calls.
- **Replay determinism preserved** — joins and records events in input order, so a recording captured during a parallel run replays byte-identical to a serial run. [crates/axon-cli/tests/stage32_replay.rs](crates/axon-cli/tests/stage32_replay.rs) forces completion order to be inverse-of-input and proves the recorded transcript is still input-ordered.
- **Narrow public hooks on `Interpreter`** — [crates/axon-runtime/src/eval.rs](crates/axon-runtime/src/eval.rs): `recording_active`, `replay_active`, `pop_replay_model_call`, `record_model_call`, `precheck_budget`, `debit_budget_for`, `caps_have`. Lets the host scheduler reuse the same recording/replay/budget bookkeeping `ask` already uses, without making the interpreter's internals public.
- **Testing-only `mock_model_slow(text, ms)`** host builtin — a `SleepingProvider` that sleeps then returns deterministic text. Lets the acceptance test be honest about overlapping waits.

**Honest scope.** This is the *first slice*, not the whole migration:
- `eval.rs` is still synchronous. Every host binding outside `flow_parallel_asks` still calls `provider.complete` on the calling thread.
- The plumbing (Arc, Send+Sync, public hooks) is in place. Subsequent stages can move individual primitives (e.g. `parallel { ... }`, `select`, `for await`) onto the async substrate without further upheaval.

### §32.2 Mechanically applicable fixes — `Diagnostic.fixes` + `axon fix`

- **`Fix` + `FixEdit` on `Diagnostic`** — [crates/axon-diag/src/lib.rs](crates/axon-diag/src/lib.rs): every diagnostic can now carry one or more `Fix { description, edits: Vec<FixEdit> }`, where each edit is a span-keyed text replacement. Empty for diagnostics the compiler can't mechanically resolve. Existing `Diagnostic::error(...)` API is unchanged — the new `fixes` field is opt-in via `.with_fix(fix)`.
- **`closest` + Levenshtein helper** — [crates/axon-tyck/src/errors.rs](crates/axon-tyck/src/errors.rs): used to power did-you-mean suggestions. Cap is `max(2, target.len()/3)`, same shape as `rustc`'s did-you-mean.
- **Three high-frequency diagnostics ship with fixes today:**
  - **E0202** (unknown name) — `name_not_found_with_candidates(span, name, candidates)`. Suggests the closest in-scope binding. Used at all three name-resolution failure sites (path heads, dotted heads, spawn target).
  - **E0203** (unknown type) — `type_not_found_with_candidates`. Suggests the closest registered type/schema/agent. Wired at the lowering site in [crates/axon-tyck/src/lower.rs](crates/axon-tyck/src/lower.rs).
  - **E0210** (effect not declared in `uses` row) — `effect_not_declared_with_fix(span, missing, declared, uses_row_inner, insert_at_body)`. Inserts the missing effect into an existing `uses { ... }` row, or synthesizes a complete clause when the function has none. Anchor positions computed by `effect_row_fix_anchors`.

- **`axon fix [--apply] [--only CODE] <file>`** — [crates/axon-cli/src/main.rs](crates/axon-cli/src/main.rs):
  - **Default = dry run.** Prints a unified diff against the source and a short summary like `fix [E0202]: replace 'greetng' with 'greeting'`. Useful in CI / code review.
  - **`--apply`** rewrites the file in place. Idempotent: rerunning on a clean file is a no-op.
  - **`--only CODE`** restricts to fixes attached to one diagnostic code. Lets `axon fix --apply --only E0202` rip through the easy did-you-means without touching effect rows.
  - **Conflict handling.** Edits to overlapping spans defer the second fix; the user reruns to pick it up.
  - **Two-pass apply.** Selects accepted hunks in source order, then splices all accepted edits in descending start-offset order in a single pass — so an early fix that *grows* the file doesn't invalidate the offsets of later fixes.
  - **Hand-rolled unified diff** (no `similar` / `difflib` dep). LCS-based, ±2 lines of context.

### Test coverage
- [crates/axon-cli/tests/stage32_async.rs](crates/axon-cli/tests/stage32_async.rs) — 5 end-to-end tests. The acceptance test (3 × 200 ms < 400 ms wall time), input-order determinism (slow → fast still arrives slow-first), capability gating (`LLM` + `Net` required), pre-dispatch validation (malformed item rejects the whole batch), and the empty-batch no-op.
- [crates/axon-cli/tests/stage32_replay.rs](crates/axon-cli/tests/stage32_replay.rs) — 2 tests. Record-then-replay round-trip is byte-identical even when completion order ≠ input order; recording captures one `ModelCall` event per ask.
- [crates/axon-cli/tests/stage32_fix.rs](crates/axon-cli/tests/stage32_fix.rs) — 8 end-to-end tests through the `axon` binary. Dry-run + apply for E0202, E0210 (existing row), E0210 (synthesized row), E0203, `--only` filtering, idempotence, no-candidate-no-fix.
- Workspace total: **939 passing**, up from 924.

### What's NOT in this stage (deliberate)
- **Full async migration of `eval.rs`.** That stays multi-week; this stage doesn't pretend otherwise. Read the §32.1 honest scope note above.
- **GBNF wired into a real local-inference backend.** The grammar emitter shipped Stage 31; routing into llama.cpp's grammar API is the next backend-bridging stage.
- **Realtime / voice flagship.** That waits on the real async runtime.
- **`axon fix --all` recursive over a project tree.** Single-file today; recursion + ignore-rules is mechanical follow-up.
- **More fix codes.** Three is enough to prove the machinery; adding fixes for E0204 (duplicate definition), E0205 (wrong arity), E0207 (no such method) follows the same `*_with_fix` constructor pattern in [crates/axon-tyck/src/errors.rs](crates/axon-tyck/src/errors.rs).

---

## Stage 31 — Depth over Breadth: Computer-Use, GBNF, Zero-Config, Error Recovery, Async Scaffolding

A focused stage answering the "stop adding spec surface" feedback: ship one flagship new capability (computer-use), three foundational depth investments (zero-config, GBNF, whole-program error recovery), and the scaffolding for the eventual async runtime — without claiming the async runtime itself is rewritten.

### §35 Computer-use primitives (flagship)
- New crate [crates/axon-computer/](crates/axon-computer/) — `Computer` capability + typed click/type/screenshot/scroll/drag/key/wait tools + a deterministic `MockDriver` whose action log makes every test byte-stable.
- `ComputerDriver` trait so real backends (Playwright, CDP, macOS AT-SPI, Win32) plug in as separate crates. The shipped driver returns an 8-byte PNG signature + a fill byte for screenshots; real pixels are a downstream crate's job.
- **Tainted-flow ready** — screenshots return `tainted: true` so the host wraps the bytes in `Tainted<Image>` at the language boundary; pixel contents can't enter a `system:` prompt without an explicit sanitize step.
- **Allowlist key validation** — single chars + a curated set (`enter`, `tab`, `esc`, …, `F1`..`F24`); unknown keys (and out-of-bounds clicks, oversized type strings, `wait > 60s`) error before they reach the backend.
- 10 host bindings: `computer_screenshot`, `computer_click`, `computer_double_click`, `computer_mouse_move`, `computer_drag`, `computer_scroll`, `computer_type`, `computer_key`, `computer_wait`, `computer_action_log`.
- `Computer` is added to the standard default capability set so `axon run` drives the mock driver without flags.

### §56.3 Native constrained decoding (schema → GBNF)
- New module [crates/axon-tyck/src/gbnf.rs](crates/axon-tyck/src/gbnf.rs) — walks any `schema` AST and emits a GBNF grammar a constrained-decoding backend (llama.cpp / mlx / vllm) can enforce token-by-token.
- Covers the cross-backend JSON subset: `string`/`integer`/`number`/`boolean`/`null`; records as fixed-key objects; `List<T>` / `Set<T>` as JSON arrays; `T?` as `T | null`; `Tainted<T>` / refinements / effect-suffix wrappers pass through transparently.
- Stable JSON-value prelude (`ws`, `string`, `integer`, `number`, `boolean`, `value`, `array`, `object`) is appended once so the grammar is portable.
- Host binding `schema_to_gbnf(name)` consults a per-program schema mirror populated at load time by `register_schemas(&program)`.

### Zero-config defaults
- **`default_model()`** builtin — returns an Anthropic provider bound to `claude-opus-4-7` when `ANTHROPIC_API_KEY` is set, otherwise a mock model with a clear "no API key set; run `axon login anthropic` for a real model" response. A bare program can `ask default_model() { user: q }` and just run.
- **Post-run footer** — `axon run` prints `─── axon run: 1.23s · N tokens · $0.0X` to stderr when stderr is a TTY (suppressed in CI/piped output and by `$AXON_NO_FOOTER`). Drives off the existing cost ledger so it's always accurate.
- **`axon run` with no path defaults to `.`** — scaffolded projects' "next: axon run" instruction works without an argument.

### Whole-program error recovery
- **Cascade suppression** in [crates/axon-parser/src/parser.rs](crates/axon-parser/src/parser.rs) — `error()` now drops any diagnostic emitted while the parser is still at the same token position as the previous error, plus exact-message+exact-span duplicates. The classic "one bad token, ten cascade lines" pattern is now one error.
- **Top-level item resync** — when an item parse fails or emits errors, `recover_to_next_item()` walks forward to the next item-start keyword (`fn`, `agent`, `tool`, `type`, `schema`, …) or contextual ident (`test`, `eval`, `policy`). Brace depth is deliberately *not* tracked — when the broken body had unbalanced braces, depth-tracking would walk to EOF.
- Result on a hand-crafted multi-error file: 14 cascade errors → 8 distinct errors. The parser keeps going and parses the subsequent items.

### Async runtime scaffolding (foundation only — NOT the rewrite)
- New crate [crates/axon-async/](crates/axon-async/) — tokio multi-thread runtime wrapper + typed `Task<T>` join/cancel handle + `AsyncMailbox<T>` with `Block` / `DropOldest` / `DropNew` backpressure policies matching the spec's `Stream<T>`.
- `AsyncRuntime::with_wall_budget(ms, fut)` for `with budget(wall = 30s)` enforcement; `join_all(futs)` for structured-concurrency `parallel { … }`.
- 8 unit tests — including `concurrent_tasks_actually_overlap` which proves 4×50ms tasks complete in <180ms (vs ~200ms serial). That's the proof point for the eventual rewrite.
- **What this is NOT**: it doesn't migrate `eval.rs` to async. That's a multi-week rewrite — every host binding, every channel, every model call. The scaffolding is here so when the migration happens, the contract is settled.

### Test coverage
- `crates/axon-computer` — 10 unit tests (screenshot tainting, in-bounds click, oob rejection, type validation, key allowlist, drag endpoints, wait cap, scripted-frames, ordered action log, JSON round-trip).
- `crates/axon-async` — 8 unit tests (spawn/join, join_all order, wall-budget timeout, mailbox block/drop_new/drop_oldest, task cancel idempotence, **concurrent_tasks_actually_overlap**).
- `crates/axon-tyck::gbnf` — 7 unit tests (empty schema, primitives, list, option, tainted pass-through, prelude presence, unknown-type fallback).
- `crates/axon-cli/tests/stage31_depth.rs` — 9 end-to-end tests through the `axon` binary.
- Workspace total: **924 passed, 0 failed** (up from 890).

### Honest scope: what's NOT in this stage
- **Real async runtime** — eval.rs is still synchronous. The scaffolding lets the migration land cleanly when it does. That migration is genuinely a multi-week project.
- **Real browser/desktop drivers** — only the deterministic mock backend ships in-tree. A Playwright/CDP driver is a separate crate.
- **Real local-inference integration** for the GBNF grammar — the emitter is done; routing the grammar into llama.cpp / mlx / vllm / etc. is per-backend wire work.
- **`axon fix --all`** — the catalogue is in place (Stage 30 §57) but applying structured fix-edits needs each diagnostic to carry rewrite candidates first.

---

## Stage 30 — Native Syntax + Developer Experience

Move the spec's own idioms onto real parser syntax (no host-binding workarounds), and ship the developer-experience surface §57–§64 promises.

### Native syntax (§19, §30, §35.2, §64.1)
- **`try { … } recover |e| { … }`** (§19) — new `ExprKind::TryRecover` (distinct from the `?` Try postfix). Runs the body in a child scope; on any runtime error binds the message string to the recover lambda's first parameter and evaluates the recover branch. `return`/`break`/`continue` propagate unchanged. Tyck joins body + recover-branch types. VM + WASM emit a clean "interpreter only" diagnostic. Wired through every match site.
- **`policy NAME { allow/deny/budget/rate/audit }`** (§30) — `parse_item` now dispatches the contextual `policy` keyword into a structured `PolicyDecl { clauses: Vec<PolicyClause> }` (Rule with optional `when`, Budget with money/token caps, Rate with `N per <duration>`, Audit). At load time the CLI compiles each decl into an `axon-guard::PolicyBlock` and registers it in the policy registry, so `policy_block_check` / `…_charge` / `…_audit_summary` work against natively-declared policies. Default-deny per §30.
- **`extern <lang> "path:fn"`** tool bodies (§35.2) — `parse_tool_decl` accepts a bare-ident ABI (`extern python "tools/s.py:run"`) in addition to the original quoted form. Runtime gains a `BridgeDispatcher` hook (held behind `Rc<RefCell<…>>` so load-time tool closures read the dispatcher installed later at run time); axon-cli wires it to `axon-ffi::bridges`, marshaling args to a JSON object keyed by param name and parsing the JSON result back into a Value. Verified end-to-end against a real `python3` subprocess.
- **`??` null-coalescing** (§64.1) — new `BinOp::Coalesce` with lexer support (`QuestionQuestion` token), parser entry at `(4, 5)` precedence (binds looser than `||`, tighter than assignment), short-circuit eval (lhs if non-nil, else rhs), tyck joins the unwrapped lhs with rhs.
- **`?.` safe field access** (§64.1) — new `QuestionDot` token, `ExprKind::SafeField`, postfix parser case mirroring `.`, eval shorts to `nil` on a `nil` receiver, tyck wraps the field type in `Nullable` (collapsing if already nullable).
- **`it` in zero-param closures** (§64.1) — `||` lexes as `PipePipe` and dispatches to `parse_lambda`, which scans the body via `expr_uses_it` (a focused walker over the common forms). A `||` whose body references `it` gets an implicit `it` parameter so `xs.map(|| it * 2)` works; a thunk `|| 7` stays zero-param so flow-combinator usage isn't broken.
- **Lambda block bodies** — `|e| { ... }` now parses the braces as a block (not a set literal), matching the `fn (params) { ... }` form.

### §57 Diagnostics & error UX
- [crates/axon-diag/src/explain.rs](crates/axon-diag/src/explain.rs) — stable offline catalogue. Each `Explanation { code, family, title, why, fix, learn }` covers a bespoke entry for every code the compiler actually emits (E0201–E0214, E0220–E0224, E0230, E0240–E0241, E0250–E0252, E0301, E0421, E0712, P0001/P0010/P0011) plus a family-level fallback so even uncatalogued codes get a useful page.
- **`axon explain <CODE | effect:X | capability:X>`** — prints the full explanation; with no arg lists all catalogued codes.
- **`axon check --json`** — machine-readable diagnostic envelope `{ "file", "ok", "diagnostics": [{ code, severity, message, span, notes }] }` for CI/editors.
- **`axon check --explain-errors`** — appends the catalogue's `Explanation::render()` block after each emitted diagnostic, keyed by code, deduplicated.

### §58 Onboarding & scaffolding
- [crates/axon-cli/src/scaffold.rs](crates/axon-cli/src/scaffold.rs) — `axon new <name> [--template <t>]` materializes a working project from one of **8 embedded templates**: `agent`, `support`, `research`, `assistant`, `pipeline`, `webhook`, `lambda`, `skill`. Each ships an `axon.toml`, `src/main.ax`, `README.md`, and (where it makes sense) a test. **All 8 templates type-check and run cleanly** with `axon run` (integration-tested).
- **`axon tour`** — 8 embedded lessons (bindings → effects → models → tools → agents → try/recover → policies → testing/replay), each printed in-terminal with a next-lesson pointer.
- **`axon run` with no path** defaults to `.` so a scaffolded project's "next: axon run" instruction just works.

### §64.2 QoL commands
- [crates/axon-cli/src/qol.rs](crates/axon-cli/src/qol.rs):
  - **`axon stats [path]`** — counts files, lines (total + code), functions (with effect-row %), agents, actors, tools, models, memories, prompts, schemas, types, policies, tests, evals. Skips `target/` / `node_modules/` / `.git/`.
  - **`axon clean`** — removes `target/` / `doc/` / `dist/` / `out/` / `.axon-cache/`, reports MB reclaimed.
  - **`axon completions {bash|zsh|fish|pwsh}`** — emits shell-specific completion scripts covering every subcommand (including the new ones).
  - **`axon doctor`** — environment check: toolchain version, project presence, axon.toml validity, vault mode (0600 on Unix), optional bridge runtimes (`python3`, `node`).
  - **`axon run --dry-run`** — type-checks, prints what *would* run (entrypoint, active capabilities, declared models/agents/tools), exits without executing or spending money.
  - **Bare `axon`** — prints a contextual hint based on project state ("no project here — try `axon new`", "you have tests — try `axon test`", etc.) instead of a wall of help.

### Test coverage
- `crates/axon-diag::explain` — 7 unit tests (known/unknown codes, family map, concept docs, one-liner format).
- `crates/axon-cli::scaffold` — 6 unit tests (every template has axon.toml + .ax file, 8 templates present, lessons sequential, name validation, substitution token used).
- `crates/axon-cli::qol` — 4 unit tests (completion shells, dir-size missing-path, which-finds-sh, stats tally).
- `crates/axon-cli/tests/stage30_devex.rs` — 14 end-to-end tests through the `axon` binary: try/recover round-trip, `??`+`?.` semantics, `it` closure + thunk coexistence, `policy { … }` enforcement, extern Python tool bridge, `axon explain`, `check --json`, `check --explain-errors`, `axon new` for all 8 templates (each scaffolds and runs), `axon tour`, `axon stats`, `run --dry-run` doesn't execute, `axon completions` for all 4 shells, `axon doctor`.
- Workspace total: **890 passed, 0 failed** (up from 859).

### CLI demo (real run)
```
$ axon run examples/stage30_native_syntax.ax
---- §19 try / recover ----
25
-1
recover message:
panic: cannot divide by zero
---- §64.1 ?? and ?. ----
42
7
city: Paris
nil-safe missing? true
---- §64.1 it closures ----
42
7
---- §30 policy block (enforced) ----
kb.search allowed? true
payments.charge allowed? false
refund when amount<=50 fails the guard? false
audit allow/deny: 1 2

$ axon explain E0712
E0712 — effect not granted by policy
  family: Capabilities, policies, taint
  why:  A policy-bound agent attempted an effect its policy doesn't allow.
        Policies are runtime-enforced guardrails that application code cannot bypass.
  fix:  Add an `allow` rule to the policy (optionally with a `when` clause),
        or route the effect through an agent that holds the grant.

$ axon new my-bot --template support
created `my-bot` from template `support`
  Tools + policy + tests — a guarded support bot.
next:
  cd my-bot
  axon run            # type-checks + runs (mock model, no keys needed)
  axon test           # runs the included test
```

---

## Stage 29 — `Result<T, E>` + `try_recover` (§19), `Stream<T>` + `for_await` (§28), `@restart` Variants (§29.7), `axon prof --cost` (§31.2)

The four sub-features the final coverage audit surfaced as partial. All shipped end-to-end.

### §19 `Result<T, E>` + `try_recover`
- `Result<T, E>` is now a typed builtin container ([crates/axon-tyck/src/builtins.rs](crates/axon-tyck/src/builtins.rs) + [crates/axon-tyck/src/lower.rs](crates/axon-tyck/src/lower.rs)). Lowers to `Dyn` at the type layer; the runtime carries ok/err via the existing `result_ok` / `result_err` / `result_is_ok` / `result_is_err` / `result_value` / `result_error` host bindings.
- `try_recover(action, on_err)` host binding mirrors the spec's `try { ... } recover |e| { ... }` block — runs `action()`, and on any runtime error passes the message string to `on_err` and returns its result. No parser change needed.

### §28 `Stream<T>` runtime + `for_await`
- [crates/axon-runtime/src/stream.rs](crates/axon-runtime/src/stream.rs) — typed `StreamHandle { buffer, capacity, closed, sent, taken, dropped, policy }` with three `BackpressurePolicy` variants:
  - **`Block`** — producer's `send` returns `Backpressure`; caller retries.
  - **`DropOldest`** — eject oldest buffered value, push the new one; `dropped` counter increments.
  - **`DropNew`** — silently drop the new value when full.
- `is_done()` distinguishes "closed and drained" from "closed but value still buffered" so `for_await` loops terminate correctly.
- Host bindings: `stream_new(name, capacity, policy)`, `stream_send`, `stream_take`, `stream_close`, `stream_is_done`, `stream_stats`. `for_await(stream_name, body)` pumps every available value through `body` until the stream is done.

### §29.7 `@restart` variant validation
- [crates/axon-runtime/src/restart_policy.rs](crates/axon-runtime/src/restart_policy.rs) — typed `RestartPolicy { Permanent | Transient | Temporary }` enum with a `from_attribute_name(s)` parser that accepts both `Permanent` and `permanent` casings; anything else returns a clean error listing the three valid variants.
- `should_restart(exit_kind)` encodes the §29.7 decision table: `Permanent` always restarts, `Transient` only on `Abnormal` exit, `Temporary` never. Supervisors now wrap the AST's raw `restart: Option<Ident>` field through this validator.
- Host bindings: `restart_policy_parse(name)` and `restart_policy_should_restart(name, exit_kind)`.

### §31.2 `axon prof --cost`
- New CLI subcommand: `axon prof --cost <ledger.json> [--top N] [--profile NAME:input/output[/cached[/per_call]]]...`. Reads a `Ledger` JSON produced by the existing `cost_save` host binding, builds a `Report`, and prints:
  - total calls + total cost in dollars;
  - latency p50 + p95;
  - per-provider breakdown (calls, input tokens, output tokens, total cost);
  - top-N most expensive calls with provider, model, cost, latency, tag.
- `--profile` repeatable to attach pricing rates per provider; without any profile the report still shows token counts + latencies so `axon prof --cost ledger.json` is useful immediately.

### Host bindings (10 new names) + tyck registrations
`try_recover`; `stream_new`, `stream_send`, `stream_take`, `stream_close`, `stream_is_done`, `stream_stats`, `for_await`; `restart_policy_parse`, `restart_policy_should_restart`. All registered in [crates/axon-tyck/src/register.rs](crates/axon-tyck/src/register.rs).

### Test coverage
- `crates/axon-runtime::stream` — 8 unit tests (send-take round trip, closed-stream rejects send, block-policy backpressure, drop-oldest keeps newest, drop-new silent drop, is_done distinguishes empty vs drained, telemetry counters, JSON round-trip).
- `crates/axon-runtime::restart_policy` — 6 unit tests (six-variant parse, unknown-variant message, Permanent decision, Transient decision, Temporary never, name round-trip).
- `crates/axon-cli/tests/stage29_long_tail.rs` — 12 end-to-end tests: `Result` type annotation parses + runs, `try_recover` calls fallback on error, stream send/take/backpressure/drop-oldest/for_await, `@restart` parse + decision table, `axon prof --cost` renders report + profile spec validation.
- Workspace total: **859 passed, 0 failed** (up from 833).

### CLI demo (real runs)
```
$ axon run examples/stage29_long_tail.ax
---- §19 Result + try_recover ----
10/2 ok? true
value: 5
7/0 err? true
reason: divide by zero
recovered: 99
---- §28 Stream<T> + for_await ----
evt-1
evt-2
evt-3
drained: 3
stats: {sent: 3, taken: 3, dropped: 0, buffer_len: 0, closed: true}
---- §29.7 @restart variants ----
Permanent / Transient / Temporary
Permanent on normal -> restart? true
Transient on normal -> restart? false
Temporary on abnormal -> restart? false

$ axon prof --cost ledger.json --profile "anthropic:300/1500" --top 5
cost report from `ledger.json`
  total calls : 3
  total cost  : $0.0210
  latency p50 : 1200 ms
  latency p95 : 2400 ms
  per-provider breakdown:
    anthropic        calls=2     in=6000     out=7000     $0.0180
    openai           calls=1     in=2000     out=500      $0.0010
  top-3 most expensive calls:
    #1  anthropic/opus               $0.0150 (2400 ms) tag=agent.research
    #2  anthropic/opus               $0.0030 (1200 ms) tag=agent.research
    #3  openai/gpt-4                 $0.0010 (800 ms) tag=agent.qa
```

---

## Stage 28 — Consensus + spawn_pool (§29.5), `human` (§29.9), Policy Block (§30), Typed FFI Bridges (§35.2), `axon serve --protocol` (§35.3)

The five remaining concrete gaps surfaced by the final coverage audit land here. Every feature is production-shaped (typed, tested, host-bound, demo-walked).

### §29.5 Consensus + spawn_pool
- [crates/axon-flow/src/consensus.rs](crates/axon-flow/src/consensus.rs) — typed `Vote { voter, choice, ranking, confidence }`, `ConsensusRule { Majority | Weighted | RankedChoice }`, and `ConsensusConfig { rule, quorum_fraction, expected_voters, weights }`. Returns a `Decision { outcome, confidence, dissenting, rule, quorum_met, vote_count }`.
- All three rules implemented:
  - **Majority** — option with most votes; ties broken by first-seen-in-stream (deterministic across runs).
  - **Weighted** — vote × judge weight × confidence; confidence ratio is `winner_score / total_weight`.
  - **RankedChoice** — instant-runoff voting; rounds eliminate the lowest-scoring option until one has a majority; iteration cap protects against pathological ties.
- `flow_spawn_pool(constructor, size)` host binding calls `constructor(i)` N times and returns the pool. Synchronous today; the API matches what an async scheduler would expose.

### §29.9 `human` pseudo-agent
- [crates/axon-guard/src/human.rs](crates/axon-guard/src/human.rs) — `open_review(reg, channel, prompt, timeout, on_timeout, now_ns)` mints a fresh request via the Stage 27 `ApprovalRegistry`. `resolve(reg, id, now_ns)` sweeps timeouts first so callers always see the *current* state. `cancel(reg, id)` denies the request with reason `"cancelled by orchestrator"`.
- Tool name `"human:review"` so audit-log scans can tell apart programmatic and human-routed approvals.
- Host bindings: `human_request`, `human_resolve`, `human_cancel`.

### §30 Policy block runtime
- [crates/axon-guard/src/policy_block.rs](crates/axon-guard/src/policy_block.rs) — typed `PolicyBlock` with:
  - `EffectKind { Tool | Net | Fs | Llm | Memory }` indexed `ClauseRule { pattern, when, action }` lists. First match wins; if no rule matches, `default_action` applies. Patterns support `*`, `prefix.*`, and exact-string.
  - `GuardClause { kind, arg, direction }` for input/output guards (`prompt_injection`, `pii`, `toxicity`, `grounded`).
  - `BudgetClause { scope, max_usd, max_tokens, max_wall_secs, window_secs, spent_*}` with running spend tracking; `charge(scope, usd, tokens)` debits. Budget-exhausted check returns label `"budget_exceeded"`.
  - `RateClause { scope, max_calls, window_secs, recent_call_ns }` with sliding-window rate limiting; auto-trims expired entries on every check.
  - Audit log — every `check_effect` call appends an `AuditEntry`; `audit_summary()` returns `(allow_count, deny_count)`.
- Order of enforcement matches §30.1: rule match → rate limit → budget headroom → audit.
- 8 host bindings: `policy_block_new`, `policy_block_allow`, `policy_block_deny`, `policy_block_check`, `policy_block_charge`, `policy_block_add_budget`, `policy_block_add_rate`, `policy_block_audit_summary`.

### §35.2 Typed FFI bridges (Python / Node / Wasm / gRPC)
- [crates/axon-ffi/src/bridges.rs](crates/axon-ffi/src/bridges.rs) — `BridgeKind` + `BridgeSpec { target, entrypoint, timeout_ms, launcher_override }`. Default launchers: `python3`, `node`, `wasmtime`, `grpcurl` — overridable per call so ops can pin a specific binary.
- Wire contract is deliberately minimal: one JSON line of args on stdin, one JSON line on stdout shaped `{"ok":true,"value":...}` or `{"ok":false,"error":"..."}`. Anything else is a typed `ProtocolViolation`.
- `call_bridge(&spec, args_json)` runs the subprocess under the same wall-clock + capability sandbox as `ffi_call` from Stage 16. Failures map to typed `BridgeError { Ffi, Bridge { message, stderr }, ProtocolViolation }`.
- Single host binding: `ffi_bridge_call(kind, target, entrypoint, args_json, timeout_ms)` returns `{ok, value_json, error}`.

### §35.3 `axon serve --protocol mcp|openai|grpc|a2a`
- [crates/axon-deploy/src/protocols.rs](crates/axon-deploy/src/protocols.rs) — `ServeProtocol` enum + pure `route(proto, request, default_handler, well_known_card_body)` function that returns `ProtocolAction::{ Reply{status, body, content_type} | Dispatch{handler, prompt, jsonrpc_id} }`. Five protocols:
  - **plain** — POST /invoke → handler (mirrors Stage 17).
  - **mcp** — JSON-RPC 2.0; `tools/list` returns the registry's tool list; `tools/call` dispatches with `params.name` as the handler; unknown methods return `-32601`.
  - **openai** — POST /v1/chat/completions; translates `messages[]` into a `role: content` prompt string; wraps the reply as `choices[0].message.content`.
  - **grpc** — POST /Service/Method; dispatches with `Service.Method` as the handler name. Plus `render_grpc_proto(service_name, handlers)` emits an `agents.proto` next to the deploy bundle.
  - **a2a** — GET /.well-known/agent-card.json returns the Stage 25 auto-published card body; POST /agent dispatches to the handler.
- `wrap_response(proto, reply, jsonrpc_id)` translates handler output back into the wire shape per protocol.
- CLI: `axon serve --protocol P` validates `P` and exports `AXON_SERVE_PROTOCOL=P` so the handler can dispatch via `serve_protocol_route` / `serve_protocol_wrap`.
- 3 host bindings: `serve_protocol_route`, `serve_protocol_wrap`, `serve_render_grpc_proto`.

### Host bindings (16 new names) + tyck registrations
`flow_consensus`, `flow_spawn_pool`; `human_request`, `human_resolve`, `human_cancel`; `policy_block_*` (8 names); `ffi_bridge_call`; `serve_protocol_route`, `serve_protocol_wrap`, `serve_render_grpc_proto`. All registered in [crates/axon-tyck/src/register.rs](crates/axon-tyck/src/register.rs).

### Test coverage
- `crates/axon-flow::consensus` — 7 unit tests (majority + tie-break, weighted with high-weight voter, ranked-choice elimination, below-quorum, weighted confidence with per-vote scores, empty votes).
- `crates/axon-guard::human` — 5 unit tests (open routes to channel, resolve reflects approval, resolve sweeps timeouts, cancel marks denied, unknown id false).
- `crates/axon-guard::policy_block` — 8 unit tests (allow-then-default-deny, wildcard patterns, when clause gating, rate-limit denial, budget exhaustion, audit log accumulation, deny over default-allow, JSON round-trip).
- `crates/axon-ffi::bridges` — 6 unit tests (kind aliases, launcher names, response parsing, bridge-error surfacing, protocol violation, JSON round-trip).
- `crates/axon-deploy::protocols` — 11 unit tests (flag parsing, plain dispatch, mcp tools/list + tools/call + unknown method, openai message translation + wrap response, a2a well-known + dispatch, grpc path parsing, render_grpc_proto).
- `crates/axon-cli/tests/stage28_orchestration_safety.rs` — 14 end-to-end tests through the `axon` binary.
- Workspace total: **833 passed, 0 failed** (up from 782).

### CLI demo (real run)
```
$ axon run examples/stage28_panel_policy_protocols.ax
---- §29.5 spawn_pool + consensus ----
panel size: 4
majority outcome: ship
dissenting: 1
---- §29.9 human pseudo-agent ----
opened: pending via slack:#treasury
---- §30 policy block ----
kb.search allowed? true
issue_refund(when ok)? true
issue_refund(when failed)? false
payments.charge allowed? false
after $0.60 spent on a $0.50 budget: budget_exceeded
audit allow/deny: 2 3
---- §35.2 FFI bridges (shape only) ----
bridge ok? false
---- §35.3 protocol adapters ----
mcp tools/list: reply status=200
openai dispatch handler: main
openai wrap status: 200
grpc proto starts: syntax = "proto3";
```

---

## Stage 27 — `@approval` (§25.6), Prompt `@version` (§24.3), `axon schema migrate` (§17.1 / §36)

The final v1.0 punch list closes here. Three small, fully production-shaped features.

### §25.6 `@approval` tool attribute
- [crates/axon-guard/src/approval.rs](crates/axon-guard/src/approval.rs) — `ApprovalRegistry` with typed `ApprovalRequest { id, tool, args_json, by, timeout_secs, on_timeout, state, actor, reason, escalated_to, requested_at_ns }`. Four `ApprovalState`s (Pending / Approved / Denied / TimedOut) and three `OnTimeout` policies (Deny — safe default, Allow — low-stakes, Escalate — re-emit to another approver).
- `sweep_timeouts(now_ns, escalation_target_for)` walks every pending request and applies the configured directive in one pass. Caller supplies a closure that picks the escalation target so the registry stays transport-agnostic — the host wires the actual Slack/email/agent delivery.
- Round-trips through JSON so pending approvals survive process restarts.
- 8 host bindings: `approval_open`, `approval_approve`, `approval_deny`, `approval_get`, `approval_pending_count`, `approval_sweep_timeouts`, `approval_next_id`, `approval_purge_terminal`.

### §24.3 Prompt `@version` registry
- [crates/axon-runtime/src/prompt_version.rs](crates/axon-runtime/src/prompt_version.rs) — `PromptVersionRegistry` keyed by `(prompt_name, version)`. First registration becomes the prompt's default; `set_default(prompt, version)` promotes another revision after eval data backs it. `pick(prompt, Some(version) | None)` returns the entry; `versions_for(prompt)` returns chronological history; `prompts()` lists every registered name. Duplicate `(name, version)` registrations are rejected so re-runs don't silently overwrite.
- 5 host bindings: `prompt_version_register`, `prompt_version_set_default`, `prompt_version_pick`, `prompt_version_versions_for`, `prompt_version_prompts`.

### §17.1 / §36 `axon schema` CLI
- `axon schema inspect <store.json> [--schema NAME]` — walks a JSON tree, finds every `{"__schema": "...", "__version": N, ...}` value, and reports counts per `(schema, version)` so operators can see the migration backlog before they trigger anything.
- `axon schema migrate <store.json> --to N [--schema NAME] [--apply]` — plans the step chain for every out-of-date entry; reports `PLAN <schema> vM -> vN steps=[...]` per entry plus a summary; refuses to downgrade (`WOULD-DOWNGRADE`) and counts each blocked entry. The `--apply` path requires a runtime-installed migrator (use `axon run <migrator-script.ax>` with the existing `schema_migrate` host binding from Stage 18) — bails out with a clean error rather than silently no-op-ing.

### Tyck registrations
13 new host names registered in [crates/axon-tyck/src/register.rs](crates/axon-tyck/src/register.rs) so programs type-check immediately: `approval_*` (8 names), `prompt_version_*` (5 names).

### Test coverage
- `crates/axon-guard::approval` — 10 unit tests (open + approve, deny + reason, double-approve rejection, unknown request, three timeout directives, empty-field validation, purge_terminal, JSON round-trip).
- `crates/axon-runtime::prompt_version` — 9 unit tests (default seeding, explicit pick, `set_default` promotion, unknown prompt/version errors, duplicate rejection, empty-name validation, chronological versions_for, prompt name dedup+sort, JSON round-trip).
- `crates/axon-cli/tests/stage27_final_gaps.rs` — 11 end-to-end tests through the `axon` binary: approval open-approve-deny, timeout-with-deny, timeout-with-escalate, prompt-version register-pick-promote, unknown-prompt error, `axon schema inspect` (count by schema/version), `axon schema migrate --to` planning, `WOULD-DOWNGRADE` blocking, `--apply` requires-runtime-migrator error.
- Workspace total: **782 passed, 0 failed** (up from 752).

### CLI demo (real runs)
```
$ axon run examples/stage27_final_gaps.ax
---- §25.6 approval gate ----
opened: state=pending
approver=treasury@example.com
after approve: state=approved actor=alice
denied: state=denied reason=needs more review
timed-out requests: 1
---- §24.3 prompt @version ----
registered prompts: [support_answer]
versions of support_answer: 3
default version: v1
post-promotion default: v3
v3 notes: added off-topic refusal

$ cat > /tmp/store.json <<'EOF'
{
  "alice": {"__schema": "Profile", "__version": 1, "name": "Alice"},
  "bob":   {"__schema": "Profile", "__version": 2, "name": "Bob"},
  "carol": {"__schema": "Profile", "__version": 3, "name": "Carol"}
}
EOF
$ axon schema inspect /tmp/store.json --schema Profile
  Profile v1: 1
  Profile v2: 1
  Profile v3: 1
$ axon schema migrate /tmp/store.json --schema Profile --to 3
  PLAN Profile v1 -> v3 key=alice steps=[1, 2]
  PLAN Profile v2 -> v3 key=bob   steps=[2]
axon schema migrate: 2 entries to upgrade, 1 already at v3, 0 blocked
```

---

## Stage 26 — `[features]` Gating (§7.1), MCP Tool Declarations (§25.5), Deterministic Test Helpers (§39.2)

Final long-tail closure: the three remaining "minor" PLAN items.

### §7.1 `[features]` conditional compilation
- [crates/axon-project/src/features.rs](crates/axon-project/src/features.rs) — `resolve_features(table, requested, enable_default)` returns the transitive closure of the user's requested feature set (Cargo-compatible shape: `default = [...]` is auto-enabled unless `--no-default-features`). `filter_program(program, active)` strips top-level items whose `#[cfg(feature = "X")]` predicate doesn't match.
- Accepted shapes for the predicate: `#[cfg(feature("name"))]`, `#[cfg(feature = "name")]`, and a bare `#[cfg("name")]`. Unrecognized predicates leave the item untouched so adding new conditions later doesn't silently strip code.
- `axon run` / `axon test` accept `--features F1,F2,...` and `--no-default-features` flags.
- `features_active()` host binding returns the resolved set to running programs so they can branch at runtime alongside compile-time gating.

### §25.5 MCP server declarations in `axon.toml`
- [crates/axon-project/src/mcp.rs](crates/axon-project/src/mcp.rs) — `McpRegistry::from_manifest_tools(tools)` resolves every `[tools.<namespace>]` block into a typed `McpTool { namespace, name, description, input_schema, provider }`. Three provider shapes:
  - `tools = [{name, description, input_schema}]` — inline declarations, available immediately.
  - `mcp = "https://..."` — remote MCP endpoint; namespace recorded in `deferred_namespaces` until a wire driver lands.
  - `mcp_command = "node mcp-fs/index.js"` — subprocess MCP server; also deferred.
- `McpClient` trait (`list_tools` / `call_tool`) with concrete `StaticMcpClient` driver returning the inline tool table — useful for tests and for declaring tools entirely in `axon.toml` without a separate process.
- Host bindings: `mcp_load_from_toml(path)`, `mcp_list_tools(namespace)`, `mcp_call_tool(namespace, name, args_json)`, `mcp_namespaces()`, `mcp_deferred_namespaces()`.

### §39.2 Deterministic testing helpers
- `mock_model(name, response)` — already shipped in Stage 6.
- New `clock_freeze(ns)` / `clock_unfreeze()` thread-local frozen clock override for `time_now` ([crates/axon-runtime/src/builtin.rs](crates/axon-runtime/src/builtin.rs)).
- New `rand_seed(seed)` sets the internal xorshift64 RNG state, making `random_int` / `random_float` byte-reproducible across runs with the same seed.
- All three exposed as host bindings without disturbing the existing `Time` / `Random` capability gates — the helpers don't require capabilities to *set* the state; they just override what the gated functions return.

### Host bindings (10 new names) + tyck registrations
`clock_freeze`, `clock_unfreeze`, `rand_seed`, `mcp_load_from_toml`, `mcp_list_tools`, `mcp_call_tool`, `mcp_namespaces`, `mcp_deferred_namespaces`, `features_active`. CLI flags `--features F1,F2,...` and `--no-default-features` added to `axon run` and `axon test`.

### Test coverage
- `crates/axon-project::features` — 6 unit tests (default seeding, transitive closure, no-default-features, unknown name, empty table, deduplication).
- `crates/axon-project::mcp` — 6 unit tests (inline registration order, deferred namespaces, static client list+call, unknown tool, empty table).
- `crates/axon-cli/tests/stage26_minor_gaps.rs` — 7 end-to-end tests through the `axon` binary: clock freeze identity, rand seed determinism, MCP load + list + call + deferred, `features_active` default seeding, `--no-default-features`, and `#[cfg(feature = "X")]` item stripping (call to a gated helper succeeds only when the feature is on).
- Workspace total: **752 passed, 0 failed** (up from 733).

### CLI demo (real run)
```
$ axon run examples/stage26_features_mcp_helpers.ax
---- §39.2 deterministic helpers ----
frozen clock identical?
true
same seed, same draw?
true
unfrozen clock advances?
true
---- §25.5 MCP server declarations ----
registered namespaces: 0
deferred-transport namespaces: 0
---- §7.1 feature introspection ----
active features (single-file demo - empty): 0

$ axon run --features redis,my-feature examples/some_project/
# `#[cfg(feature = "redis")]` items are now in scope; default features still seeded
```

---

## Stage 25 — Closing the Long Tail: Context Policy, Sagas, Durable Timers, Grounding, Media Gen, Skill Use, Auto-Publish, AxVM C ABI, Metrics, Serverless

Stage 25 closes every remaining PLAN gap that surfaced after the Stage 24 sweep: nine self-contained features, each fully tested + host-bound.

### §27.3 Context policy
- [crates/axon-runtime/src/context_policy.rs](crates/axon-runtime/src/context_policy.rs) — `ContextPolicy { on_overflow: OverflowStrategy, max_tokens, reserved_for_response }` with four strategies:
  - `SummarizeOldest { with, target_ratio }` (default — emits which old messages the caller should feed to a cheap summarizer);
  - `DropOldest` (sliding window; protects `system` and the last conversational turn);
  - `DropLeastRelevant { score_fn_id }` (host-evaluated relevance scorer);
  - `Error` (fail closed — refuse to call the model when the prompt won't fit).
- `ContextPolicy::plan(messages)` returns a `ContextOutcome { kept, removed, action, original_tokens, final_tokens, budget }` — bookkeeping only; the host actually performs any summarize call.
- Conservative token-count heuristic (`chars / 4`, ceil) when the provider hasn't returned a real counter.

### §52 Sagas with compensations
- [crates/axon-flow/src/saga.rs](crates/axon-flow/src/saga.rs) — `SagaStep { name, action, compensate? }` and `run_saga(input, steps)` running every action forward; on the first failure compensations run **LIFO** against the values their respective forwards produced. Status `committed | compensated | aborted`, audit trail per step (`Succeeded | Failed | Compensated | CompensationFailed | Skipped`). Compensation errors don't halt the rollback — they're recorded and the next compensation runs.

### §52.2 Durable timers (`sleep_until`)
- [crates/axon-trigger/src/durable_timer.rs](crates/axon-trigger/src/durable_timer.rs) — `DurableTimerTable` keyed by string id; persists via JSON round-trip through `axon-memory`. `arm(timer)`, `cancel(id)`, `due(now_ns)` (sorted by deadline), `mark_fired(id)`, `purge_fired_or_cancelled()`. Survives process restarts; the runtime loads on startup and only fires timers whose wall-clock deadline has actually passed.

### §50.2 / §50.3 RAG grounding & citations
- [crates/axon-rag/src/grounding.rs](crates/axon-rag/src/grounding.rs) — `CitationPassage`, `Citation`, `assess_grounding(answer, passages, citations, config)` → `GroundingReport { claims, citations, grounded_fraction, citation_validity, passed }`. Per-claim sentence split + lexical overlap against supporting passages; per-citation existence + span-overlap check. Configurable `min_overlap`, `grounded_threshold`, `citation_threshold`. Stop-word filter so "the cat is" isn't penalized for empty content.

### §51.2 / §51.3 Multimodal generation
- [crates/axon-media/src/generate.rs](crates/axon-media/src/generate.rs) — `MediaProvider` trait with default `generate_image` / `generate_audio` returning `Unsupported`; concrete `MockProvider` produces a valid PNG signature + WAV RIFF header so end-to-end plumbing works without a real backend. Typed `GenerateImageRequest { prompt, width, height, format, negative_prompt, seed, n }` / `GenerateAudioRequest { prompt, voice, sample_rate, format, max_duration_secs, seed }` with bounds validation (dims ≤4096, n ≤8, sample_rate 8000..=192000).

### §53 Skill use + effect narrowing
- [crates/axon-skill/src/use_skill.rs](crates/axon-skill/src/use_skill.rs) — `bind_skill(manifest, caller_caps, alias?)` → `SkillBinding { alias, caller_caps, skill_caps, effective_caps, is_satisfied, missing_caps }`. `narrow_effects(callee_caps, caller_caps)` intersects rows. `explain_missing(binding)` returns a stable diagnostic listing the missing rows. Importer-with-fewer-caps cannot accidentally hand a Net-needing skill to a pure caller.

### §54.1 Agent card auto-publication
- [crates/axon-a2a/src/auto_publish.rs](crates/axon-a2a/src/auto_publish.rs) — `derive_agent_card(summary, base_url)` produces a verified `AgentCard` from a compile-time `AgentSummary { name, version, description, handlers, auth, schemas }`. Endpoint follows `<base>/agent`. `render_well_known(card)` returns the pretty-printed JSON body the serve layer returns from `/.well-known/agent-card.json`.

### §35.4 AxVM C ABI
- [crates/axon-vm-cabi/](crates/axon-vm-cabi/) — new `staticlib + cdylib + rlib` crate. `axvm_compile(source)` → opaque `AxvmHandle*`, `axvm_set_caps(handle, "Console,Net")`, `axvm_call_main(handle, &mut out_json)` returns the result as a JSON string the caller frees with `axvm_free_string`. Per-thread `axvm_last_error()`. C header at [crates/axon-vm-cabi/include/axvm.h](crates/axon-vm-cabi/include/axvm.h). Embed Axon in any language with a C FFI by linking `libaxvm.{a,so,dylib}`.

### §41 `/metrics`, serverless render
- [crates/axon-deploy/src/metrics.rs](crates/axon-deploy/src/metrics.rs) — `MetricsRegistry` with atomic counters (requests_total, success, error, bytes_in, bytes_out, handler_us_total, in_flight, uptime). `render_prometheus()` emits standard 0.0.4 text-exposition format with `# HELP` / `# TYPE` blocks for every metric.
- [crates/axon-deploy/src/serverless.rs](crates/axon-deploy/src/serverless.rs) — `ServerlessTarget { Lambda | GcpFunction | CfWorker }` driven from `#[lambda]` / `#[gcp_function]` / `#[cf_worker]` attribute hints. `render_lambda_yaml` emits an AWS SAM template; `render_gcp_function_yaml` emits a GCF spec; `render_cf_worker_toml` emits a wrangler.toml. The trampoline carries memory + timeout + env so `axon deploy --target=cf_worker` can produce ready-to-deploy scaffolding next to the `.axskill` archive.

### §43 Formal EBNF grammar
- [spec/grammar.ebnf](spec/grammar.ebnf) — normative ISO/IEC 14977 EBNF for every top-level item, statement, expression, pattern, and literal, including agent / actor / supervisor / graph / network / orchestrate declarations, the prompt-slot DSL, and the `Tainted<T>` / refinement type syntax.

### Host bindings (24 new names) + tyck registrations
`context_policy_plan`, `flow_saga_run`, `timer_arm`/`cancel`/`due`/`mark_fired`/`pending_count`/`save`/`load`, `rag_assess_grounding`, `media_generate_image`/`audio`, `skill_bind`/`narrow_effects`, `agent_card_derive`/`well_known_path`, `metrics_record`/`render_prometheus`, `serverless_render`.

### Test coverage
- `crates/axon-runtime::context_policy` — 6 tests.
- `crates/axon-flow::saga` — 5 tests.
- `crates/axon-trigger::durable_timer` — 7 tests.
- `crates/axon-rag::grounding` — 6 tests.
- `crates/axon-media::generate` — 6 tests.
- `crates/axon-skill::use_skill` — 6 tests.
- `crates/axon-a2a::auto_publish` — 6 tests.
- `crates/axon-vm-cabi` — 6 tests (compile, run, set_caps, errors, drop-NULL).
- `crates/axon-deploy::metrics` — 3 tests; `::serverless` — 6 tests.
- `crates/axon-cli/tests/stage25_end_to_end.rs` — 9 end-to-end tests through the `axon` binary.
- Workspace total: **733 passed, 0 failed** (up from 666).

### CLI demo (real run)
```
$ axon run examples/stage25_completeness.ax
---- §27.3 context policy ----
action: drop_oldest
kept: 2
removed: 2
---- §52 saga with compensations ----
refunded:payment-99
released:seat-42
saga status: compensated
---- §52.2 durable timers ----
pending: 2
due now: payroll
pending after fire: 1
---- §50.2 RAG grounding ----
grounded fraction: 0.5
passed? false
---- §51 multimodal generation ----
first image bytes: 8
---- §53 skill_bind narrows effects ----
satisfied? false
error: skill `scraper` requires capabilities the importer doesn't hold: Fs.Write
---- §54.1 agent card auto-publish ----
well-known path: .well-known/agent-card.json
---- §41 metrics + serverless ----
metrics has counter? true
lambda yaml mentions axskill? true
```

---

## Stage 24 — Multi-Agent Orchestration (§29), Reasoning & Planning (§49), Trajectory Eval (§55), Cost/Latency Optimization (§56)

The "intelligence layer" lands as four production-quality pillars on top of Axon's existing language + agent infrastructure.

### §29 Multi-agent orchestration
- **`axon-flow::network::Network`** — declarative agent topology with `OneWay` and `Bidirectional` edges. Verification runs three structural checks: edges reference known nodes, no cycles (DFS with full cycle-path reconstruction), and reachability from any root via BFS. Bidirectional edges deliberately expand to two one-way edges so the cycle detector catches the `critic <-> writer` deadlock the spec uses as its canonical hazard.
- **`axon-flow::graph::WorkflowGraph`** — explicit DAG of typed steps (§29.6). Kahn's-algorithm topological order with deterministic intra-layer alphabetical ordering, plus `roots()` / `leaves()` for entry/exit attribution. The host's `flow_graph_run` schedules nodes in topo order, threading each node's predecessors' results into the node's callable as a record so users can write graphs without learning a new scheduling API.
- **`axon-flow::debate`** — two personas argue, a judge decides (§29.8). Each round runs `pro(question, transcript)` then `con(question, transcript)` then accumulates the typed `Statement` into the transcript so positions can sharpen across rounds; the final `judge(question, transcript)` returns the verdict.
- **`axon-flow::tree_of_thought`** — beam-search over branched candidate steps (§29.8, §49.2). At each depth level, every surviving thought spawns children via `expand`; children are scored by `score`; the top-`width` (by score) survive. Non-finite scores are clamped to `f64::NEG_INFINITY` so NaN doesn't corrupt the heap.

### §49 Reasoning & planning
- **`axon-runtime::reasoning::ReasoningBudget`** — first-class thinking-token budget (§49.1) distinct from I/O-token budgets. `Effort { Low | Medium | High | Adaptive }`, `max_thinking_tokens`, `expose` flag controlling whether the reasoning trace is returned as a `Tainted<Stream<Thought>>` to UIs. `ReasoningBudgetStack` mirrors the existing `BudgetStack` so a child scope can't escape a parent reasoning ceiling.
- **`axon-flow::strategy::PlanningStrategy`** — pluggable `plan` loop shapes (§49.2): `ReAct | PlanExecute | Reflexion { rounds } | TreeOfThought { width, depth } | Debate { rounds } | Custom { step_id }`. Tagged-JSON-serializable so strategies round-trip through `axon trace` and `axon replay`.
- **`axon-flow::strategy::DirectiveOnError`** — typed `on_step_error` directives (§49.4): `Backoff { secs } | Replan { hint } | Repair | FinalizeBest | Escalate { to } | Abort`. Each strategy interprets the directive in its own loop shape.
- **`plan_react_loop(max_steps, think, act, observe)`** — host-side ReAct driver. Returns a list of `{ step_index, thought, action, observation }` records; observe returns `{ observation, done }` and the loop stops as soon as `done = true`.
- **`axon optimize <prompt.ax> --eval <suite.ax> [--trials N]`** (§49.6) — searches the cartesian product of `// VARIANT:` swap points against the eval suite, scores each combo via `axon check` + the suite's pass count, writes the winner as `<name>.vN.ax`. Makes "prompt engineering" a measured, gated, reviewable change instead of guesswork.

### §55 Trajectory eval, red-teaming, simulation
- **`axon-eval::trajectory::Trajectory`** — typed view of a recorded run with `steps[]` (each step holding optional `tool_call`, `observation`, `error`), `allowed_tools`, `forbidden_tools`, `optimal_steps`, and the final `answer`. Pure-function metrics over this struct:
  - `tool_accuracy` — fraction of tool calls that named an allowed tool and didn't error.
  - `step_efficiency` — clamped ratio of optimal-steps to actual-steps.
  - `recovered_from_errors` — true iff an error step is followed by a non-error step.
  - `no_forbidden_tool_called` / `no_secret_exposed(secrets)` — safety predicates.
  - `grounded_in_observations` — fraction of answer-claims (sentence-split) that appear verbatim in some step's observation.
- **`axon-eval::redteam`** — curated adversarial suites (§55.2). `redteam_suite("std:injection" | "std:jailbreak" | "std:tool_abuse" | "std:exfiltration" | "std:pii_trap" | "std:all")` returns typed `RedteamCase { id, category, payload, watched_tools, secrets, assertion }`. The `assertion` is structural (`NoToolCalled` / `NoSecretExposed` / `AnswerOmitsCanary` / `Refuses`) so the host evaluates the *behavioural* outcome — the capability/policy/taint layers must still prevent unsafe acts even if the model is talked into trying.
- **`axon-eval::sim::World`** — deterministic simulation harness (§55.3). Owns a virtual clock (`clock_ns`), a seeded splitmix64 PRNG (same seed → same draws byte-identically), a list of `AgentBox`es with FIFO mailboxes and a scripted action stream (`Send { to, payload } | Note { kind, payload } | Settle`), and an event log. `World::advance(dt_ns)` steps every agent once in name order; `World::run_until(dt_ns, max_ticks, predicate)` runs until the predicate fires or the tick cap hits.

### §56 Cost & latency optimization
- **`axon-cost::cache::PrefixCache`** — provider-side prompt-prefix cache shadow (§56.1). FNV-1a keyed entries with `(tokens, inserted_at, expires_at, hits)`; `lookup` increments hits and tokens-saved telemetry; expired entries are swept on the miss path. `CacheStats { lookups, hits, misses, tokens_saved, entries, hit_rate() }` flows into `cost_cache_stats()` so the agent program can show cache effectiveness in `axon prof --cost`.
- **`axon-flow::race`** — speculative cheap-then-deep execution (§56.3). Runs candidates in order until one is `accept`-ed; returns `{ winner_index, value, considered, accepted }` so callers can measure "what fraction of queries actually needed the expensive model."
- **`axon-flow::batch`** — issue N independent inputs through one step (§56.3). The synchronous shape preserves the API a future async batched executor would expose, so call sites don't change.
- **`axon-flow::route::DifficultyRouter`** — heuristic difficulty-routed model selection (§56.4). `estimate_difficulty(prompt, thresholds)` returns `Trivial | Normal | Hard` based on length + question-mark count + hard-keyword triggers (`prove`, `derive`, `step by step`, `compare and contrast`, ...). Conservatively biased toward `Hard` — wrong direction wastes the user's time more than wasting money.

### Host bindings (35 new names)
`flow_network_new` / `flow_network_add_node` / `flow_network_add_edge` / `flow_network_verify` / `flow_network_unreachable_from`; `flow_graph_new` / `flow_graph_add_node` / `flow_graph_add_edge` / `flow_graph_verify` / `flow_graph_topo` / `flow_graph_roots` / `flow_graph_leaves` / `flow_graph_run`; `flow_debate` / `flow_tree_of_thought` / `flow_race` / `flow_batch`; `flow_estimate_difficulty` / `flow_route_difficulty`; `reasoning_budget_new` / `reasoning_budget_debit` / `reasoning_budget_status`; `plan_react_loop`; `eval_trajectory_new` / `eval_trajectory_add_step` / `eval_trajectory_set_answer` and six metric queries; `redteam_load` / `redteam_refusal_phrases`; `sim_world_new` plus 9 world manipulation bindings; `cost_cache_insert` / `cost_cache_lookup` / `cost_cache_stats` / `cost_cache_clear`.

### Test coverage
- `crates/axon-flow` — 46 unit tests (network, graph, debate, ToT, race, batch, route, strategy, refine, sequential, parallel).
- `crates/axon-cost::cache` — 7 prefix-cache tests.
- `crates/axon-runtime::reasoning` — 7 reasoning-budget tests.
- `crates/axon-eval` — trajectory metrics (10), redteam suites (5), sim.World (6).
- `crates/axon-cli/tests/stage24_orchestration.rs` — 6 end-to-end tests through the binary.
- `crates/axon-cli/tests/stage24_reasoning_eval.rs` — 7 end-to-end tests.
- `crates/axon-cli/tests/axon_optimize.rs` — 3 CLI tests for `axon optimize`.
- Workspace total: **666 passed, 0 failed** (up from 579).

### CLI demo (real run)
```
$ axon run examples/stage24_orchestration_and_eval.ax
---- §29 multi-agent orchestration ----
network with bidi edge ok?
false
error: network has cycle: critic -> writer -> critic
graph verify ok?
true
topological order: [classify, retrieve, draft, review]
debate verdict: decision: ship now
---- §49 reasoning & planning ----
400 tokens breached? false
400+700 breached? true
react steps logged: 1
---- §55 trajectory eval + redteam ----
tool_accuracy: 0.666...
recovered_from_errors: true
red-team injection suite size: 3
---- §56 cost & latency optimization ----
hit / hit / miss: true / true / false
cache stats: {lookups: 3, hits: 2, ..., hit_rate: 0.666...}
race winner: cheap:hello
routed tier: hard
```

---

## Stage 23 — Dynamic-library FFI (§35) + Delegated Identity (§54.2) + `axon pkg` (§36)

Three production-quality additions land together in Stage 23: in-process FFI for pre-built native libraries, the `on_behalf_of` delegation primitive that lets one principal authorize another, and a CLI subcommand for managing project dependencies.

### §35 Dynamic-library FFI via `libloading`
- New `axon-ffi::dlib` module: `DynamicLibrary` (RAII wrapper around `libloading::Library`), `DlibValue { I64 | F64 | Str }`, typed `DlibError`.
- `DynamicLibrary::open(path)` — load any `.so` / `.dylib` / `.dll` from disk; symbols looked up lazily via `Library::get`.
- `DynamicLibrary::call(symbol, args, ret_is_str)` — dispatches based on the arg shape against a small, deliberately narrow set of supported C signatures: i64 arity 0..=4 → i64, f64 arity 0..=2 → f64, single `*const c_char` → `*const c_char`, void → `*const c_char`. Anything outside that closed set is rejected statically with `DlibError::UnsupportedSignature` rather than risking undefined behavior.
- Host binding `ffi_dlib_call(lib_path, symbol, args_list, ret_is_str)` returns `{ ok, value, error }`. Args are tagged records `{ ty: "i64"|"f64"|"str", v: <val> }` so the host can pick the right C signature without relying on Axon's dynamic-typing inference.
- Real test: opens `libSystem.dylib` (macOS) / `libm.so.6` (Linux), calls `cos(0.0)`, asserts `1.0` round-trips through the boundary.

### §54.2 Delegated identity (`on_behalf_of`)
- New `axon-a2a::identity::Delegation` — `{ principal, audience, scopes, expires_at_secs, nonce }`. Serializes to canonical JSON; the JSON is what's actually signed.
- `SignedDelegation { delegation_json, signature_hex, signer_pubkey_hex }` — same shape as `SignedAgentCard` from Stage 22, reusing the Ed25519 primitives.
- `KeyPair::sign_delegation(&Delegation)` produces a `SignedDelegation`; `SignedDelegation::verify(&TrustStore, expected_audience, now_secs)` is **fail-closed** in this order:
  1. hex parse (signature & pubkey both syntactically valid);
  2. trust-store membership check (untrusted-but-mathematically-valid signatures rejected);
  3. signature verification (ed25519-dalek);
  4. audience match (`expected_audience` must equal `delegation.audience`);
  5. expiry check (`now_secs <= delegation.expires_at_secs`).
- The trust check runs *before* the signature math so an attacker probing with a known-untrusted key learns nothing about signature internals — defense-in-depth against timing oracles.
- Host bindings: `a2a_sign_delegation(seed_hex, principal, audience, scopes_list, expires_at_secs, nonce, dest_json_path)` returns the signer's pubkey hex; `a2a_verify_delegation(signed_path, trust_store_name, expected_audience, now_secs)` returns the parsed delegation record on success and errors on any failed gate.
- Reuses the Stage 22 `TRUST_STORES` thread-local registry — store-name semantics are identical between signed cards and signed delegations.

### §36 `axon pkg` subcommand
- New `axon pkg <list|add|remove|audit>` for read/edit of the `[deps.<name>]` tables in `axon.toml`.
- `pkg list` prints each dep with its path; `pkg add NAME --path P` writes (or overwrites) the entry, validating the dep name is alphanumeric/underscore/dash only; `pkg remove NAME` deletes the entry (errors if missing); `pkg audit` walks every declared dep and reports `ok`, `WARN` (dir present but no `axon.toml` or `src/`), or `FAIL` (path doesn't exist).
- Round-trips through `toml::Value` so unknown manifest sections survive edits unchanged. `--manifest PATH` overrides the default of `./axon.toml` so the command works from any directory.
- Network/git deps land in a later stage; today the surface is local-path only — same constraint as `axon-project` itself.

### Test coverage
- `crates/axon-ffi/src/dlib.rs` — 5 unit tests including a real `cos(0.0)` invocation through `libloading`.
- `crates/axon-a2a/src/identity.rs` — 5 new delegation tests on top of Stage 22's signed-card tests (round trip, audience mismatch, expiry, untrusted signer, tampered JSON).
- `crates/axon-cli/tests/host_dlib_and_delegation.rs` — 6 end-to-end tests through the `axon` binary.
- `crates/axon-cli/tests/axon_pkg.rs` — 8 tests covering list / add / remove / audit on real on-disk manifests.

### CLI demo (real run)
```
$ axon run examples/stage23_dlib_and_delegation.ax
ffi_dlib_call(cos, 0.0) ok =
true
cos(0.0) =
1
signed delegation written to
/tmp/axon-stage23-deleg.json
verified delegation:
  principal =
user:alice
  audience  =
research-agent-1
  nonce     =
demo-nonce-001
```

---

## Stage 1 — Lexer, Parser, AST

**Crates:** [axon-lexer](crates/axon-lexer/), [axon-parser](crates/axon-parser/), [axon-ast](crates/axon-ast/), [axon-diag](crates/axon-diag/)

- Hand-written lexer with Unicode NFC normalization on identifiers.
- Nested block comments (`/* ... /* ... */ ... */`).
- Doc comments (`///` item-attached, `//!` module-level) preserved as distinct token kinds.
- String literals: plain, raw (`r"..."`), multi-line (`"""..."""`), and prompt strings (`p"..."`).
- Domain literals: money (`$1.99`, `€10`), duration (`30s`, `5m`, `2h`), dates (`2026-05-19`), times (`14:30`).
- Numeric literals: integer, float (with exponents), hex/binary/octal, underscores for grouping.
- Recursive-descent parser with Pratt expression parsing for full operator precedence.
- Items: `fn`, `type`, `schema`, `agent`, `actor`, `tool`, `model`, `memory`, `prompt`, `trait`, `const`, plus imports and visibility (`pub`).
- Every AST node carries a `Span { file: u16, start: u32, end: u32 }` — file-stamped for cross-file diagnostics.
- Rich diagnostics with primary + secondary labels, source-file registry, ANSI-colored terminal rendering.

## Stage 2 — Type Checker

**Crate:** [axon-tyck](crates/axon-tyck/), [axon-types](crates/axon-types/)

- Bidirectional type checking: synthesis + checking modes.
- Full effect rows on every function arrow (`uses { Network, FileSystem }`).
- `Tainted<T>` as a distinct type (not a subtype of `T`) — propagation is explicit.
- Schema types with structural records, optional fields, default values.
- Generics on functions, types, traits.
- Trait resolution with coherence and overlap checking.
- Union types, refinement predicates, narrowing.
- Type-error suggestions ("did you mean...") for unknown identifiers and fields.

## Stage 3 — Tree-Walking Interpreter

**Crate:** [axon-runtime](crates/axon-runtime/)

- Direct AST evaluation for the pure-Rust subset.
- Closures with proper lexical scoping.
- Pattern matching (literals, tuples, records, variants, wildcards, guards).
- Mutable local bindings (`var`) and `while`/`for` loops.
- User-defined functions, recursion, higher-order functions.
- Built-in operations on `Int`, `Float`, `String`, `Bool`, lists, records, options.

## Stage 4 — Capability System

**Crate:** [axon-runtime](crates/axon-runtime/) (caps module)

- Static effect rows checked at compile time.
- Per-frame capability attenuation: a function can only pass *strictly fewer* capabilities to its callees.
- No ambient authority — `with caps { ... }` is the only way to grant a capability.
- Built-in capabilities: `Console`, `Network`, `FileSystem`, `Random`, `Clock`, `Env`.
- Capability-aware standard library (e.g., `print` requires `Console`, `http.get` requires `Network`).

## Stage 5 — AxVM Bytecode VM

**Crate:** [axon-vm](crates/axon-vm/)

- Stack-based bytecode with ~40 opcodes.
- Lua-style upvalues for closure capture (open/close on stack frame exit).
- Compile target shared with the interpreter — programs run identically on both.
- Inline caches for record-field access.
- Tail-call optimization for `return f(...)` in tail position.

## Stage 5.5 — Actors

**Crate:** [axon-runtime](crates/axon-runtime/) (actors module)

- `actor` declarations with typed state, message handlers, and lifecycle hooks (`on_start`, `on_stop`).
- Synchronous-dispatch mailbox model (deterministic, replayable).
- Typed messages with pattern-matched handlers.
- Cross-actor send returns a future / awaited reply.
- Supervision with restart strategies.

## Stage 6 — LLM Integration

**Crate:** [axon-models](crates/axon-models/)

- Anthropic Messages API client via `ureq`.
- `model "claude-..." { ... }` declarations with system prompt, temperature, top_p, max_tokens.
- `prompt "..."` blocks with template slots and structured output (`-> Schema`).
- Multi-turn tool-use loop: the model can call back into Axon tools and the runtime feeds results back in.
- Streaming responses (server-sent events) for token-by-token output.

## Stage 6.5 — Tools

**Crates:** [axon-runtime](crates/axon-runtime/), [axon-models](crates/axon-models/)

- First-class `tool` declarations with typed parameters, typed result, capability requirements.
- Tools are exposed to LLM models as JSON-schema tool definitions automatically.
- Tool-call results round-trip through the model's tool-use protocol.
- Capability checking on tool calls — a model can't invoke a tool requiring caps the caller doesn't hold.

## Stage 7 — Tracing, Budgets, Replay

**Crate:** [axon-runtime](crates/axon-runtime/) (tracing/budget/replay modules)

- Structured JSON event log: every model call, tool call, capability grant, actor message.
- `with budget(tokens=1000, cost=$0.50, time=30s) on_exceeded handler` — composable budget stacks.
- Record mode: capture every non-deterministic outcome (LLM responses, randomness, clock reads, network).
- Replay mode: re-execute against the captured tape — byte-identical results.
- Trace viewer JSONL format compatible with downstream tooling.

## Stage 8 — Modules, Tests, Project Manifest

**Crate:** [axon-project](crates/axon-project/)

- File-as-module project layout (`src/foo.ax` → module `foo`).
- `axon.toml` manifest: `[package]`, `[run]`, `[caps]`, `[dependencies]`.
- Public/private item visibility with cross-module checking.
- `#[test]` attribute + `axon test` runner with per-test isolation.
- Module collision diagnostic (P0001), unknown-module diagnostic (P0011).
- Re-export support and import paths (`use foo.bar.baz`).

## Stage 8.5 — Privacy, Per-File Spans, Dependencies

**Crates:** [axon-diag](crates/axon-diag/), [axon-project](crates/axon-project/)

- Source registry with `file_id` stamped onto every span — diagnostics point to the right file across the whole project.
- Privacy diagnostic (P0010): accessing a private item from another module is a hard error with a fix-it hint.
- Dependency resolution from `axon.toml` — local path deps work today; registry deps stubbed for v1.

## Stage 9 — WebAssembly Backend

**Crate:** [axon-wasm](crates/axon-wasm/)

- WASM codegen via `wasm-encoder` for the pure-Int subset (`fn`, arithmetic, `if`, `while`, `let`/`var`, recursion).
- Two-pass body compiler to handle locked locals.
- `wasmparser` validation of every emitted module.
- `wasmi` execution path for running compiled WASM in-process.
- `axon build <file> -o out.wasm` CLI command.
- Host-import surface for `print_int` and similar.

## Stage 9.5 — Language Server (LSP)

**Crate:** [axon-lsp](crates/axon-lsp/)

- LSP server via `lsp-server` + `lsp-types`.
- Push diagnostics on file change (parse + type errors).
- Hover: type signature + doc comment for the item under the cursor.
- Go-to-definition for local and cross-module symbols.
- Completion: keywords, in-scope identifiers, member access.
- Editor integration ready (VS Code, Neovim, any LSP-aware client).

## Stage 22 — Platform Sandboxes (Linux seccomp + macOS sandbox-exec) + Ed25519 A2A Identity

Real OS-level isolation for tool subprocesses, plus verifiable cross-org agent identity via Ed25519.

### §42 Platform sandboxes
- New `axon-sandbox::platform` module with `PlatformProfile` (declarative intent: `read_only_fs`, `allow_network`, `allow_subprocess`, `extra_syscalls`) and `PlatformSandbox` (mutates a `std::process::Command` before spawn).
- Three presets: `strict()` (default — read-only FS, no net, no fork), `networked()`, `build_tool()`.
- **Linux**: seccomp-bpf filter via `seccompiler` (pure Rust, same library Firecracker uses). `PR_SET_NO_NEW_PRIVS + seccomp(2)` installed inside `pre_exec`, so the filter is in force from instruction 1 of the user's process. Whitelist covers POSIX core syscalls; `KillProcess` (SIGKILL) on anything outside the allowlist.
- **macOS**: `sandbox-exec(1)` wrapping. The command is rewritten as `sandbox-exec -p <inline-sbpl> <original-program> <args...>`. sbpl profile is `(deny default)` + opt-ins for the operations the profile enables.
- **Windows**: documented v0 limit — `Limits` + wall-timeout still apply, Job Object integration deferred.
- Host binding `sandbox_run_with_profile(program, args, cpu, mem_mb, wall_s, profile_name)` returns the same result record as `sandbox_run` from Stage 15; the kernel sandbox layer is additive.

### §54 Ed25519 signed agent identity
- New `axon-a2a::identity` module: `KeyPair` (ed25519-dalek), `SignedAgentCard { card_json, signature_hex, signer_pubkey_hex }`, `TrustStore`, `IdentityError`.
- `KeyPair::generate()` uses OS RNG; `KeyPair::from_seed_bytes` is deterministic for reproducible tests and seed-vault recovery.
- `KeyPair::sign_card(&AgentCard)` produces a `SignedAgentCard` with the canonical card JSON, 64-byte hex signature, and 32-byte hex verifying key.
- `SignedAgentCard::verify(&TrustStore)` is **fail-closed**: signature math must verify *and* the signer's pubkey must be in the trust store. Untrusted-but-mathematically-valid signatures are rejected with `IdentityError::Untrusted(hex)`.
- `KeyPair::Debug` redacts the private seed; the only way to extract it is through `seed_hex()` / `seed_bytes()` (named so audit trails flag the call).
- Host bindings: `a2a_keypair_generate`, `a2a_keypair_from_seed`, `a2a_sign_card(card_json_path, seed_hex, signed_dest_path)`, `a2a_verify_signed_card(signed_path, trust_store_name)`, `a2a_trust_store_new(name, list_of_pubkey_hex)`.

### CLI demo (real run)
```
$ axon run demo.ax
--- platform sandbox: strict profile ---
hello-from-sandbox                          # echo ran under macOS sandbox-exec
0
--- Ed25519 keypair ---
a79b45a4162df95e                            # first 16 hex chars of pubkey
64                                          # 64-char seed
--- sign + verify ---
research-acme                               # verified card.agent_id
Acme Research Agent
--- signed card on disk ---
{
  "card_json": "{...canonical AgentCard JSON...}",
  "signature_hex": "68edfe57...a04",        # 64-byte Ed25519 sig
  "signer_pubkey_hex": "a79b45a4...500"     # 32-byte verifying key
}
```

---

## Stage 21 — OAuth Vault, TLS Serve, Graceful Shutdown, `axon login`

Production security layer: refreshable OAuth tokens, rustls-terminated HTTPS, SIGINT-driven drain shutdown, and a CLI credential capture flow.

### §40.2 OAuth-aware vault
- New `axon-secret::oauth::OauthToken` — `access_token` + optional `refresh_token` + `expires_at_secs` + `token_url` + `client_id`. All fields serialize for vault storage; the value layer keeps `Secret<T>` redaction.
- `is_expired_at(now)`, `needs_refresh(slack_secs)` — pure predicates for tests.
- `OauthToken::refresh()` — POSTs `grant_type=refresh_token` to the stored `token_url` via `ureq`, parses the JSON response, rotates `access_token` (and `refresh_token` if the server rotated it), and recomputes `expires_at_secs` from `expires_in` (defaults to 1 hour per OAuth2 spec).
- `Vault::set_oauth(name, &token)` / `Vault::get_oauth(name)` — OAuth tokens live under `oauth:{name}` in the same vault JSON, so plain API keys and OAuth tokens coexist without a schema change.
- `Vault::load_oauth_with_refresh(name, slack_secs, path)` — loads, refreshes if needed, persists rotated token back to disk before returning. The standard "always-fresh, no re-login" pattern.
- Typed `TokenRefreshError`: `NoRefreshToken` / `NoTokenUrl` / `Http(...)` / `HttpStatus { status, body }` / `Io` / `Parse`.

### §41 TLS via rustls
- `Server::with_tls_pem(cert_pem_path, key_pem_path)` — loads a PEM-encoded cert chain + private key (rustls-pemfile parses RSA, ECDSA, EdDSA, and PKCS8 keys). Pure Rust, no OpenSSL.
- Single crypto provider registered via `rustls::crypto::ring::default_provider().install_default()` (called lazily on first use).
- `handle_connection_plain` / `handle_connection_tls` share the same `read_request_from<S: Read>` / `write_response_to<S: Write>` helpers, so adding TLS didn't fork the protocol logic.
- v0 limit: TLS reads PEM from disk at bind time; cert rotation requires a redeploy. ACME / file-watching rotation is a Stage 22+ enhancement.

### §41 Graceful shutdown
- `Server.in_flight: AtomicUsize` — incremented when a connection thread starts, decremented when it returns. After `stop` flips, `run()` waits up to `shutdown_grace` (default 10s) for `in_flight == 0` before returning.
- `Server::install_signal_handler()` — Unix-only: installs a `SIGINT`/`SIGTERM` handler that flips `stop`. Process-wide; latest install wins (last `axon serve` invocation in the same process). Non-Unix is a no-op — callers can still flip `server.stop` programmatically.
- `serve_run` / `serve_run_tls` both call `install_signal_handler()` before entering the run loop, so `Ctrl-C` and `kill -TERM` drain in-flight requests automatically.

### §36 `axon login`
- `axon login <provider> [--vault PATH] [--key VALUE]` — stores an API key in the vault under `<PROVIDER>_API_KEY`.
- Key source precedence: `--key` arg → `<PROVIDER>_API_KEY` env var → interactive stdin prompt.
- Vault path resolution: `--vault` flag → `AXON_VAULT` env → `~/.axon/vault.json`.
- Vault file is mode `0600` on Unix (verified by `axon-secret::Vault::save`).
- Multiple `axon login` calls on the same vault append cleanly — keys for different providers coexist.

### `axon serve` extended
- New flags: `--tls-cert PATH` / `--tls-key PATH` (must be paired). When present, routes through `serve_run_tls` instead of `serve_run`.
- Banner now mentions Ctrl-C: `axon serve [tls]: listening on https://… (Ctrl-C to shutdown)`.

### CLI demo (real run)
```
$ axon login anthropic --vault /tmp/v.json --key sk-ant-demo
saved `ANTHROPIC_API_KEY` to /tmp/v.json (mode 0600 on Unix)
$ ls -l /tmp/v.json
-rw-------  1 user user  72 May 19 21:35 /tmp/v.json

$ axon serve svc.ax --listen 127.0.0.1:18432 \
                    --tls-cert /tmp/cert.pem --tls-key /tmp/key.pem &
axon serve [tls]: listening on https://127.0.0.1:18432 (Ctrl-C to shutdown)

$ curl -sk -X POST https://127.0.0.1:18432/invoke -d "hello-axon"
got: hello-axon

$ kill -TERM %1
(server drains in-flight requests, exits cleanly)
```

---

## Stage 20 — OTLP Exporter + `axon replay/--patch` + `axon trace` + `axon repl`

Closes three observability/tooling gaps: OpenTelemetry-compatible trace export, deterministic replay-with-edits, and an interactive REPL.

### §31 OTLP/HTTP-JSON exporter
- New `axon-runtime::otlp` module: converts internal `TraceSpan` records into OpenTelemetry Protocol JSON (`ExportTraceServiceRequest` shape) — byte-compatible with what real OTLP exporters POST to `/v1/traces`.
- Span-kind mapped to `SPAN_KIND_INTERNAL` (1); error spans get `STATUS_CODE_ERROR` (2) with the message; OK spans get `STATUS_CODE_OK` (1).
- Resource bag includes `service.name`, `telemetry.sdk.name`, `telemetry.sdk.language`, and `telemetry.sdk.version`.
- Stable 32-hex-char `traceId` keyed off the recording's smallest span id so record/replay pairs produce identical IDs.
- Nanosecond timestamps (OTLP requires nanos; we multiply our `start_ms`/`end_ms` by 1_000_000).
- Host binding `trace_export_otlp(path, service_name)` — NativeExt that pulls live spans via `Interpreter::with_trace_spans`.
- Refuses cleanly when tracing wasn't enabled: `trace_export_otlp: tracing is not enabled — re-run with axon run --trace …`.

### §32 `axon replay <rec> <src>` with `--patch`
- New CLI subcommand. Strict mode (default) replays an Axon program against a recording byte-identically and reports `consumed N of M recorded event(s)` on stderr.
- `--patch` mode flips `Replay::lenient`: a program edited *after* the recording was made (extra model calls, etc.) gets a cleaner `replay exhausted (patch mode)` error message and the cursor report shows `[patch]` vs `[strict]`.
- New `Interpreter::enable_replay_lenient` + `replay_progress() -> Option<(cursor, total, lenient)>` for end-of-run summaries.

### §36 `axon trace <file.jsonl>`
- Reads a JSONL trace file (the format `--trace PATH` already writes) and pretty-prints a colored span tree with durations, kinds, and any attached error.
- Empty file → `(no spans)`; malformed JSON → typed error with the offending line number.
- No external deps — uses `serde_json::Value`.

### §36 `axon repl`
- Interactive read-eval-print loop with banner, prompt-numbered input, and three dot-commands: `.help`, `.quit`/`.exit`, `.effects`.
- Each line is wrapped in a synthesized `fn __repl_N() uses { ...standard row... } { ... }` so built-ins like `print_int`, `time_now`, `http_fetch` work without `uses` clauses in REPL input.
- Persistent interpreter — bindings from `let x = ...` survive across prompts (within the synthesized fn's scope).
- Tracing is auto-enabled so the REPL can show effect summaries on demand.

### CLI demo (real run)
```
$ axon run --trace internal.jsonl --record rec.json svc.ax
the answer is 42

$ axon trace internal.jsonl
trace: 1 span(s), max span duration 0ms
ask (ask)  0ms

$ head -5 traces.otlp.json
{
  "resourceSpans": [
    {
      "resource": {
        "attributes": [ ... "service.name": "demo-svc" ... ]

$ axon replay rec.json svc.ax
axon replay [strict]: consumed 1 of 1 recorded event(s)

$ axon replay rec.json svc-edited.ax --patch
... replay exhausted (patch mode): no recorded event remaining for this call
axon replay [patch]: consumed 1 of 1 recorded event(s)

$ printf 'print_int(40 + 2)\n.effects\n.quit\n' | axon repl
Axon 0.1.0 REPL — type `.help` for commands, `.quit` to exit.
axon[1]> 42
axon[2]> active capabilities: {Audit, Channel, Console, ...}
```

---

## Stage 19 — `for await`, `select`, `plan with` Enhancements

Three core language constructs from §14 and §26 wired through parser, type checker, and runtime.

### §14 `for await` — async-stream iteration
- New `is_await: bool` field on `ExprKind::For`; parser recognizes `for await pat in expr { body }`.
- Eval routes async-flagged iteration to a Chan-draining loop when the iterator is a `Chan` value; lists, sets, maps, tuples, and strings still iterate as usual.
- `break` and `continue` work correctly mid-stream.
- The `await` keyword is semantic markup today (the synchronous interpreter drains eagerly) — the surface is identical to what the async scheduler will use when it lands.

### §14 `select` — multi-channel arms
- Replaced the raw-text `SelectArm` placeholder with a typed AST: `SelectArmKind::{Recv, Timeout, Else}`.
- New parser for `select { name = recv(chan) => body, _ = timeout(dur) => body, else => body }`. No new tokens — uses existing call syntax instead of `<-` so it composes with the rest of the grammar.
- Runtime semantics: walk recv arms in declaration order, pick the first whose channel is non-empty. If none is ready, take the first `timeout` (fires immediately in the sync runtime), then the first `else`. With no fallback arm, a runtime error surfaces.
- Declaration order is the tiebreak when multiple channels are ready — deterministic and easy to reason about.

### §26 `plan with` — `max_steps` + `output: Schema`
- `max_steps:` slot now actually caps the tool-use loop (overrides the default `MAX_TOOL_USE_ITERATIONS = 8`). Validates that the value is `> 0` *before* the first model call.
- `output:` slot steers the model toward emitting JSON (appended as a system-prompt nudge) and parses the final response as a `Record` so call sites can pattern-match `r.field`.
- Type checker now returns `Dyn` for `plan ... { output: ... }` (Stage 12 gradual-escape-hatch propagation) so field access on the returned record type-checks without manual ascription.
- Bad JSON in the model's final response surfaces a clean `plan` with `output:` expects valid JSON error.

### Type-checker generalizations driven by these features
- `for` over `Dyn` no longer errors (E0230) — propagates as `Dyn` element type, same as field access on `Dyn`.
- Method calls on `Dyn` no longer error — return `Dyn` with `Dyn` argument types.
- These two relaxations are what let stdlib calls (`list_new(...)`, `chan()`, `mem_*`, `rag_*`) feed into the new constructs without per-program annotations.

### CLI demo (real run)
```
--- for await over a list (sync stream) ---
1
2
3
--- for await draining a chan ---
alpha
beta
gamma
--- select picks the ready channel ---
hello-from-b
--- select falls through to else ---
(no message)
--- plan with output: schema returns a Record ---
42                                  # parsed from {"answer":"42","confidence":0.95}
```

---

## Stage 17 — Deploy (HTTP server, health checks, env binding, manifest)

**Crate:** [axon-deploy](crates/axon-deploy/). Adds `axon serve` and `axon deploy` CLI subcommands.

### `axon-deploy` (§41) — production deploy primitives
- **Minimal HTTP/1.1 server** in pure Rust (`std::net::TcpListener`, thread-per-connection, no `tokio`/`hyper`).
  - Routes `POST /invoke` to a user handler.
  - Routes `GET /healthz` (liveness: 200 if anything is up) and `GET /readyz` (readiness: 503 if any check fails).
  - 15s read/write timeouts, 4 MiB body cap, 32 KiB header cap — malformed/oversize requests are rejected before they reach a handler.
  - `Connection: close` per response; one request per socket. Keeps the server loop ~200 LoC.
- `HealthCheck` trait + built-in `Liveness` + `AlwaysHealthy(name)`. Custom checks plug in via `Server::with_check(Box::new(...))`.
- **Dotenv loader** that preserves existing process env by default (deployment-baked secrets win over repo defaults). `overwrite: true` for the rare cases that need it.
- `DeployManifest` (`deploy.json`): `name`, `entrypoint_handler`, `port`, `env: BTreeMap<String, String>`, `health_checks: Vec<String>`, optional `dotenv` + `vault` refs, version-checked on load.

### CLI bindings
- `env_get(name)`, `env_get_or(name, default)`, `env_load_dotenv(path, overwrite)`.
- `serve_run(listen_addr, handler)` — **NativeExt**; binds the server, then routes each request through the Axon handler in the interpreter thread. Cross-thread handoff via `mpsc` so the single-threaded interpreter stays sound even though the HTTP loop is multi-threaded.
- `deploy_write_manifest(dir, name, entrypoint, port)`.

### New `axon` subcommands
- **`axon serve <file> [--listen ADDR] [--handler NAME]`** — start the HTTP server.
- **`axon deploy <project_dir> -o <out_dir> [--name N] [--port P] [--handler H]`** — package: writes `<name>.axskill` (Stage 14 format) + `deploy.json`.

### CLI demo (real run, with `nc` hitting the live server)
```
$ axon deploy src_project -o dist --port 9191 --handler greet
wrote dist/greet-svc.axskill
wrote dist/deploy.json

$ axon serve server.ax --listen 127.0.0.1:9192 --handler greet
axon serve: listening on 127.0.0.1:9192

$ printf 'POST /invoke HTTP/1.1\r\n...\r\n\r\naxon!' | nc 127.0.0.1 9192
HTTP/1.1 200 OK
Content-Length: 13
Hello, axon!!

$ printf 'GET /healthz HTTP/1.1\r\n...\r\n\r\n' | nc 127.0.0.1 9192
HTTP/1.1 200 OK
{"checks":[{"detail":"","name":"liveness","ok":true}],"ok":true}
```

---

## Stage 16 — Trajectory Eval, Cost Optimization, FFI

**Crates:** [axon-eval](crates/axon-eval/), [axon-cost](crates/axon-cost/), [axon-ffi](crates/axon-ffi/)

### `axon-eval` (§55) — scenarios, metrics, suite runner
- `Scenario { name, input, expected, tags }`; `RunResult { output, latency_ms, data, error }`.
- Five built-in metrics: `ExactMatch`, `Contains`, `RegexLike` (anchored wildcard), `JsonPath` (`/foo/bar=value`), `LatencyP95` (per-suite budget over actual latencies).
- `Metric` trait is object-safe so suites hold heterogeneous `Box<dyn Metric>`.
- `Suite::run` takes `FnMut` so the host can capture `&mut Interpreter` and dispatch through user-supplied handlers.
- `SuiteReport::to_junit_xml()` emits CI-friendly XML with per-testcase failure messages.

### `axon-cost` (§56) — ledger, profiles, reports
- `CostEntry`: per-call `(provider, model, input_tokens, output_tokens, cached_input_tokens, latency_ms, timestamp_ns, tag)`.
- `ProviderProfile`: prices in cents-per-million-tokens (so integer math doesn't round cheap models to zero), plus optional per-call fixed cost and cached-input discount.
- `Ledger::save`/`load` for cross-process persistence with versioned JSON.
- `Report::build` aggregates: total calls / cents, per-provider summary, **p50 + p95 latency**, top-N most-expensive calls.

### `axon-ffi` (§35) — subprocess FFI with JSON line protocol
- `call_once(spec, request)` — spawn, write one JSON line on stdin, read one JSON line from stdout, return response. Bounded by `timeout_ms` with a sentinel thread that `SIGKILL`s overstays via libc.
- `Connection` for persistent line-protocol children (amortizes spawn cost).
- All FFI is subprocess-based — no `libloading`, no `unsafe`, no native deps beyond `libc` on Unix.

### CLI bindings
- `eval_suite_new`, `eval_add_scenario`, `eval_add_metric` (`exact_match`/`contains`/`regex_like`/`json_path`), `eval_set_latency_budget`, `eval_run` (NativeExt — invokes the user handler), `eval_report_junit`.
- `cost_record`, `cost_profile_add`, `cost_report`, `cost_save`, `cost_load`, `cost_reset`.
- `ffi_call(program, args_list, request_json, timeout_ms)` → `{ ok, response_json, error }`.

### CLI demo (real run)
```
--- eval ---
3                                  # total scenarios
2                                  # passed (1 expected-mismatch failed)
--- cost ---
3                                  # total calls
2700                               # total cents ($27.00 across 3 calls)
600                                # p50 latency_ms
900                                # p95 latency_ms
anthropic                          # top-spend provider
anthropic                          # most expensive single call's provider
--- ffi ---
true
{"hello":"axon"}                   # JSON round-tripped through /bin/cat
```
The JUnit XML report from `eval_report_junit` includes:
```xml
<testsuite name="polite-suite" tests="3" failures="1">
  <testcase name="wrong" time="0">
    <failure message="exact_match: expected `Goodbye, axon!`, got `Hello, axon!`"/>
  </testcase>
</testsuite>
```

---

## Stage 15 — Guardrails, Secrets, Sandbox

**Crates:** [axon-guard](crates/axon-guard/), [axon-secret](crates/axon-secret/), [axon-sandbox](crates/axon-sandbox/)

### `axon-guard` (§30) — guardrails for inputs and outputs
- `ContentFilter` with detectors for **Email**, **US phone**, **US SSN**, **credit cards** (with Luhn check + word boundaries), **API keys** (`sk-ant-`/`sk-`/`ghp_`/`github_pat_`), **AWS access keys** (`AKIA…`/`ASIA…`), **private-key headers** (RSA / OpenSSH / EC / DSA / PKCS8).
- `Finding` carries a `redacted` preview so logs never show the raw match.
- `injection_score(text)` — heuristic 0..=1 with weighted flags for `IgnorePrevious` / `RoleOverride` / `EmbeddedSystemTag` / `PromptLeak` / `JailbreakLingo` / `SuspiciousBase64Blob`.
- `Policy` — `allow`/`deny` rule list with `Contains` and anchored `Wildcard` matchers, default action, first-match-wins evaluation. JSON-serializable for `axon.toml` integration.

### `axon-secret` (§40) — redaction-aware secrets
- `Secret<T>` — wraps a value so `Debug`, `Display`, and `Serialize` all emit `<redacted>`. The only way to read the inner value is `expose_for_use()`, whose name flags audit-trail usage.
- `Vault` — JSON file with `{ version, secrets: BTreeMap }`. Save uses atomic `tmp → rename`; on Unix the file is created with **mode `0600`** via `OpenOptions::mode`.
- `Vault::load` **rejects insecure permissions** (`mode & 0o077 != 0` → `InsecurePermissions { path, mode }`) with an actionable error message ("Run `chmod 600 …`").

### `axon-sandbox` (§42) — resource-limited subprocesses
- `Limits { cpu_seconds, memory_mb, max_open_files, wall_seconds }` with conservative defaults (10s CPU / 256 MB / 64 FDs / 15s wall).
- `run_sandboxed` applies `setrlimit` via Unix `pre_exec` so the limits take effect **before the child's `execve`**.
- Wall-clock timeout is enforced by the parent via polling + `kill`.
- `SandboxResult` distinguishes `wall_timeout` (parent killed it) from `limit_breached` (kernel signaled, e.g. `SIGXCPU`).
- Windows path is documented as v0 limit — `Limits` are accepted but only wall timeout fires.

### CLI bindings
- `guard_scan_pii(text)`, `guard_scan_secrets(text)`, `guard_injection_score(text)`, `guard_policy_evaluate(json_path, text)`.
- `secret_open(path)`, `secret_get(name)` (returns `<redacted>` by default), `secret_set(name, value)`, `secret_remove(name)`, `secret_names()`, `secret_redact(s)`.
- `sandbox_run(program, args_list, cpu_seconds, memory_mb, wall_seconds)` → `{ exit_code, stdout, stderr, wall_ms, wall_timeout, limit_breached }`.

### CLI demo (real run)
```
--- guardrails ---
3
Email
PhoneUs
SsnUs
2                                  # 2 injection flags
allow                              # policy: "approved: ship it"
deny                               # policy: "key=AKIA..." (deny-aws-key)
--- secrets ---
2                                  # 2 secrets stored
<redacted>                         # secret_get never shows clear value
--- sandbox ---
sandboxed                          # stdout from /bin/sh
0                                  # exit_code
true                               # wall_timeout fired on `sleep 5` with 1s limit
```
The vault file is `-rw-------` on disk — `ls -l` confirms mode `0600`.

---

## Stage 14 — Durable Triggers, Skill Packaging, A2A Discovery

**Crates:** [axon-trigger](crates/axon-trigger/), [axon-skill](crates/axon-skill/), [axon-a2a](crates/axon-a2a/)

### `axon-trigger` (§52) — schedules + in-process scheduler
- `Schedule::Every { period_ns }` / `Schedule::At { when_ns }` / `Schedule::Cron(CronExpr)`.
- `CronExpr::parse` — 5-field POSIX subset (`min hour dom mon dow`) with `*`, lists, ranges, `*/N` steps, and the OR-semantics for restricted dom+dow.
- `Schedule::due_at(last, now)` — coalescing catch-up: a process offline for hours fires once on resume, not once per missed window. Returns the period-grid deadline so persisted state stays aligned.
- `Scheduler::tick(now_ns)` is pure bookkeeping — returns IDs in deterministic id-sorted order so the host can dispatch.
- `save_to_memory(store)` / `load_from_memory(store)` — durable state on top of any `axon_memory::Store`. Restart resumes exactly where the last save left off.
- `RetryPolicy` with bounded `max_attempts` + `backoff_ns`; trigger auto-disables after exhaustion.

### `axon-skill` (§53) — `.axskill` package format
- One file, one JSON document: `{ manifest, files }`. Pure-Rust, deterministic on disk, `cat | jq`-friendly.
- `Manifest`: name, version, entrypoint, capabilities, dependencies, authors, description.
- `Skill::pack(dir)` walks a directory tree (UTF-8 only) and slurps every file relative to the root.
- Every archive carries a **content hash** (FNV-1a over canonical key/body concat). `Skill::verify()` rejects tampered files, unknown format versions, missing entrypoints.
- `Skill::unpack_to(dest)` only writes after verification passes.

### `axon-a2a` (§54) — agent cards & discovery
- `AgentCard` schema matching `/.well-known/agent-card.json`: `agent_id`, `name`, `version`, `endpoint`, `capabilities` (with input/output schema URLs), `auth` (`None`/`ApiKey`/`Bearer`/`OAuth2`), `pricing`, `rate_limits`, free-form metadata.
- `AgentCard::verify()` — required fields non-empty, endpoint is `http(s)://`, capability names unique, schema URLs well-formed.
- `load_card_from_path(path)` for local/test discovery; `fetch_card(base_url)` for HTTPS via `ureq` with a 10-second total timeout.

### CLI bindings
- `trigger_every`, `trigger_at`, `trigger_cron`, `trigger_remove`, `trigger_len`, `trigger_save`, `trigger_load`, and `trigger_tick(now_ns)` (NativeExt — looks up the handler by name in the global env and invokes it).
- `skill_pack(src_dir, dest)`, `skill_install(pkg, dest_dir)`, `skill_inspect(pkg)`.
- `a2a_card_load(path)`, `a2a_card_fetch(url)`, `a2a_card_has_capability(path, name)`.

### CLI demo (real run)
```
--- triggers ---
2                                      # 2 triggers registered
[trigger] heartbeat                    # first tick: heartbeat fires
1
[trigger] running nightly report       # second tick: nightly_at fires
1
--- skill packaging ---
hello-skill
1.0.0
h_ce8bc9a61355bb08                     # content hash
--- a2a discovery ---
Acme Research Agent
https://research.acme.com/agent
2                                      # 2 capabilities
Research
true                                   # has_capability("Summarize")
```

---

## Stage 13 — Orchestration & Reasoning

**Crate:** [axon-flow](crates/axon-flow/), plus a runtime extension (`NativeExtFn` / `Value::NativeExt`).

### `axon-flow` — three production combinators
- `sequential(steps, input)` — pipeline: thread `input` through each `Step` in order; short-circuit on first failure with `sequential[i]` path crumb.
- `parallel(steps, input)` — fan-out: run every `Step` on the *same* input, collect outputs in order. `Vec<Result<...>>` so caller decides on partial failure. `parallel_strict` short-circuits.
- `refine(generate, critique, revise, accept, max_rounds)` — planner-critic loop. Keeps best-so-far; returns `(draft, score, RefineOutcome::Accepted | MaxRounds)`. Matches §49.3 `flow.reflect` shape.
- Generic over `Step<I, O>`; `FnStep` wraps closures; `ScriptedStep` returns pre-recorded outputs (for tests).
- `FlowError` carries a path breadcrumb (`sequential[2]`, `parallel[branch=4]`, `refine[critique:2]`) so failures localize to a step.

### Runtime extension: `NativeExtFn` / `Value::NativeExt`
- New native-fn variant that **takes `&mut Interpreter` plus the call-site span**. Enables host bindings to invoke user closures supplied as arguments.
- `Interpreter::register_native_ext(name, fn)` is the registration point.
- Arity, capability, and error-trace handling all mirror the existing `NativeFn` path.

### CLI bindings (all `NativeExt` so user fns can be supplied as steps)
- `flow_seq(list_of_callables, input)` → final value
- `flow_parallel(list_of_callables, input)` → `List<output>`
- `flow_refine(generate, critique, revise, max_rounds, accept_score)` → `{ draft, score, rounds, outcome }`

### CLI demo (real run)
```
--- pipeline ---
issue #42                              # classify → summarize → polish
--- fan-out ---
research n                             # style_concise
RESEARCH NOTES                         # style_loud
[summary] research notes               # style_label
--- refine ---
accepted                               # outcome
draft+++                               # draft after 3 revisions
3                                      # rounds used
```

---

## Stage 12 — RAG & Multimodal

**Crates:** [axon-rag](crates/axon-rag/), [axon-media](crates/axon-media/)

### `axon-rag` — production retrieval primitives
- `RecursiveChunker` — paragraph → sentence → word → char fallback with configurable overlap.
- `HashEmbedder` — deterministic feature-hashing (FNV-1a) + L2 normalization. Zero network, byte-identical across runs, ideal for tests/replay; same trait surface as future remote embedders.
- `Index` — in-memory vector + lexical store with stable JSON serialization; rehydrates BM25 sidecar on load; idempotent inserts keyed by content-hashed `passage_id`.
- `Bm25` — Okapi BM25+ (k1=1.5, b=0.75) over the same tokenization the embedder uses.
- `Retriever` — hybrid scorer (`α·cosine + (1-α)·BM25_normalized`); top-k with deterministic tie-break.
- CLI bindings: `rag_index_new`, `rag_ingest`, `rag_chunk`, `rag_retrieve`, `rag_save`, `rag_load`, `rag_index_len`.

### `axon-media` — typed multimodal primitives
- `Image::from_bytes` — real header parsers for **PNG IHDR**, **JPEG SOFn**, **GIF LSD**; returns width, height, MIME, byte length without decoding pixels.
- `Audio::from_bytes` — RIFF/WAVE parser; walks `fmt ` and `data` sub-chunks to recover sample rate, channels, bit depth, and duration_ms; rejects non-PCM formats with a typed error.
- `Document::from_bytes` — UTF-8 text with optional form-feed (`\x0C`) page boundaries (the `pdftotext` convention).
- `sniff()` — content-first MIME detection (PNG/JPEG/GIF/WAV/PDF/text/unknown).
- Every parser rejects malformed input with a typed `MediaError` — no panics, no decoder exploits.
- CLI bindings: `media_image_load`, `media_audio_load`, `media_document_load`, `media_sniff`.

### Host + tyck wiring
- Type checker now treats `Dyn.field` as `Dyn` so structured Records returned by native bindings can be drilled into without type ascriptions — propagation through `Dyn` and `Error`, not a blanket relaxation.
- All `rag_*` and `media_*` names added to the type checker's PURE built-in list.

### CLI demo (real run)
```axon
fn main() uses { Console } {
    let img = media_image_load("chart.png")
    print(img.mime)            // "image/png"
    print_int(img.width)        // 800
    print_int(img.height)       // 600

    rag_index_new(512)
    rag_ingest("doc1", "Ferret are small carnivorous mammals ...")
    rag_ingest("doc2", "The stock market closed lower today ...")
    let hits = rag_retrieve("how to train a ferret pet", 2)
    // top hit: "Domestic ferret can be trained to use a litter box..."
    rag_save("kb.json")
    // ... next process can rag_load("kb.json") and pick up where we left off
}
```

---

## Stage 11 — Standard Library & Memory

**Crates:** [axon-std](crates/axon-std/), [axon-memory](crates/axon-memory/)

### `axon-std` — 87 functions across 8 modules
- `std.string` (16): `str_upper`, `str_lower`, `str_trim*`, `str_split`, `str_join`, `str_contains`, `str_starts_with`/`ends_with`, `str_replace`, `str_repeat`, `str_len`, `str_chars`, `str_index_of`, `str_substring`.
- `std.list` (16): `list_new`/`len`/`push`/`pop`/`get`/`set`, `first`/`last`/`contains`, `reverse`/`sort`, `take`/`drop`/`concat`, `index_of`, `remove_at`.
- `std.map` (10): insertion-ordered KV — `map_new`/`len`/`get`/`get_or`/`set`/`remove`/`contains`/`keys`/`values`/`merge`.
- `std.set` (9): dedup-preserving — `set_new`/`add`/`remove`/`contains`/`union`/`intersection`/`difference`/`to_list`/`len`.
- `std.option` (6) + `std.result` (7): first-order helpers; `Result` uses tagged `Instance` so pattern matching works uniformly.
- `std.math` (14): `pow`, `sqrt`, `floor`/`ceil`/`round`, `sin`/`cos`/`tan`, `log`/`log2`, `exp`, `pi`/`e`, `gcd`.
- `std.time` (9): `dur_seconds`/`millis` and reverse, `date_year/month/day`, `date_make` (validates day-of-month), `date_is_leap`.

### `axon-memory` — pluggable persistent stores
- `EphemeralStore` — in-process `BTreeMap`.
- `FileStore` — JSON-backed with **atomic writes** (write `.tmp`, fsync, rename) so a partial process kill never leaves the file half-written.
- Single `Store` trait — downstream code holds `Arc<dyn Store>` and never cares which backend.
- Sorted-key snapshots → deterministic on-disk output.
- Schema versioning with explicit rejection of unknown versions.
- `forget_tagged()` and `forget_older_than()` for retention/GDPR-style passes.

### Host wiring
- `Interpreter::register_native()` is a new public method so downstream crates plug into the runtime without modification.
- `axon-cli` exposes 8 `mem_*` built-ins (open file, open ephemeral, set, get, remove, keys, len, contains) backed by `axon-memory`, with automatic `Value`↔`serde_json::Value` conversion.
- Type checker's built-in list extended so all stdlib + `mem_*` names type-check.

### CLI demo (real, end-to-end)
```axon
fn main() uses { Console } {
    mem_open_file("/tmp/wordcount.json")
    let words = str_split(str_lower(sentence), " ")
    let counts = map_new()
    // ... (frequency map via list_get/map_set) ...
    mem_set("top_word", best_word)   // persists across processes
    mem_set("top_count", best_count)
    print_int(best_count)
}
```
The JSON file survives the process exit and is read back in the next run.

---

## Stage 10 — Doc Site Generator & Formatter

**Crates:** [axon-doc](crates/axon-doc/), [axon-fmt](crates/axon-fmt/)

### `axon doc`
- Walks a `LoadedProject`, pairs `///` doc comments with the items that follow.
- Emits one HTML page per module + an index, with an embedded stylesheet.
- Markdown rendering via `pulldown-cmark`; signatures and doc bodies HTML-escaped.
- Public vs. private items get distinct CSS classes.
- Item-prefix tokens (`pub`, `async`, attributes) correctly bridged so doc comments attach to the right item.
- CLI: `axon doc <path> [-o dir]`.

### `axon fmt`
- Token-stream-based formatter — re-emits each lexer token with canonical spacing rules.
- 4-space indent tracked by `(`, `[`, `{` nesting.
- Canonical spacing for binary ops, `,`, `:`, `->`, `=>`, `|>`, unary prefixes, call-attach.
- Blank-line collapsing (runs collapse to one); always-trailing newline.
- Idempotent: `format(format(x)) == format(x)`, pinned by tests.
- CLI: `axon fmt <path> [--write] [--check]` — `--check` exits non-zero on diff (CI-friendly).

---

## CLI surface today

```
axon run    <file>           # interpret (Stage 3+)
axon test   [path]           # run #[test] items (Stage 8)
axon build  <file> -o out    # compile to WASM (Stage 9)
axon doc    <path> [-o dir]  # generate HTML doc site (Stage 10)
axon fmt    <path> [--write] [--check]   # canonical formatter (Stage 10)
axon lsp                     # language server (Stage 9.5)
```

## Test count by area

| Crate | Tests |
|---|---|
| axon-ast | 3 |
| axon-diag | 5 |
| axon-doc | 5 |
| axon-fmt | 12 |
| axon-lexer | 24 |
| axon-lsp | 7 |
| axon-models | 12 |
| axon-parser | 13 |
| axon-project | 9 |
| axon-runtime | 9 + 16 + 11 + 15 + 11 + 28 + 9 + 27 (per-module suites) |
| axon-tyck | 30 |
| axon-types | — |
| axon-vm | 12 |
| axon-wasm | — (runs via integration tests) |
| **Total** | **253 passing** |

## Workspace shape

```
crates/
├── axon-ast/        # syntax tree types + spans
├── axon-cli/        # the `axon` binary
├── axon-diag/       # diagnostics + source registry
├── axon-doc/        # static HTML doc site generator   ← Stage 10
├── axon-fmt/        # canonical formatter              ← Stage 10
├── axon-lexer/      # tokenizer
├── axon-lsp/        # language server
├── axon-models/     # LLM client + prompts + tool loop
├── axon-parser/     # recursive-descent + Pratt parser
├── axon-project/    # axon.toml + module loading
├── axon-runtime/    # interpreter + caps + actors + tracing + budgets + replay
├── axon-tyck/       # bidirectional type checker
├── axon-types/      # core type representation
├── axon-vm/         # AxVM bytecode VM
└── axon-wasm/       # WebAssembly codegen
```
