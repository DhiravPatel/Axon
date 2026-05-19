# Axon — Implemented Features

A snapshot of everything Axon ships today, grouped by the stages that introduced each capability. All features below are covered by the workspace test suite (**253 tests passing** across 27 crates).

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
